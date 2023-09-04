use std::net::SocketAddr;

use salvo::affix;
use salvo::http::header::{HeaderValue, CONTENT_TYPE};
use salvo::http::mime;
use salvo::hyper::server::conn::AddrIncoming;
use salvo::prelude::*;

use rmqtt::{
    anyhow,
    base64::{engine::general_purpose, Engine as _},
    bytes, chrono, futures, log,
    serde_json::{self, json},
    tokio::sync::oneshot,
    HashMap,
};
use rmqtt::{
    broker::types::NodeId,
    grpc::{
        client::NodeGrpcClient, Message as GrpcMessage, MessageBroadcaster, MessageReply as GrpcMessageReply,
        MessageSender, MessageType,
    },
    node::NodeStatus,
    ClientId, From, Id, MqttError, Publish, PublishProperties, QoS, Result, Retain, Runtime,
    SubsSearchParams, TopicFilter, TopicName, UserName,
};

use super::types::{
    ClientSearchParams, Message, MessageReply, PublishParams, SubscribeParams, UnsubscribeParams,
};
use super::PluginConfigType;
use super::{clients, plugin, subs};

fn route(cfg: PluginConfigType) -> Router {
    Router::with_path("api/v1")
        .hoop(affix::inject(cfg))
        .hoop(api_logger)
        .get(list_apis)
        .push(Router::with_path("brokers").get(get_brokers).push(Router::with_path("<id>").get(get_brokers)))
        .push(Router::with_path("nodes").get(get_nodes).push(Router::with_path("<id>").get(get_nodes)))
        .push(Router::with_path("health/check").get(check_health))
        .push(
            Router::with_path("clients").get(search_clients).push(
                Router::with_path("<clientid>")
                    .get(get_client)
                    .delete(kick_client)
                    .push(Router::with_path("online").get(check_online)),
            ),
        )
        .push(
            Router::with_path("subscriptions")
                .get(query_subscriptions)
                .push(Router::with_path("<clientid>").get(get_client_subscriptions)),
        )
        .push(Router::with_path("routes").get(get_routes).push(Router::with_path("<topic>").get(get_route)))
        .push(
            Router::with_path("mqtt")
                .push(Router::with_path("publish").post(publish))
                .push(Router::with_path("subscribe").post(subscribe))
                .push(Router::with_path("unsubscribe").post(unsubscribe)),
        )
        .push(
            Router::with_path("plugins")
                .get(all_plugins)
                .push(Router::with_path("<node>").get(node_plugins))
                .push(Router::with_path("<node>/<plugin>").get(node_plugin_info))
                .push(Router::with_path("<node>/<plugin>/config").get(node_plugin_config))
                .push(Router::with_path("<node>/<plugin>/config/reload").put(node_plugin_config_reload))
                .push(Router::with_path("<node>/<plugin>/load").put(node_plugin_load))
                .push(Router::with_path("<node>/<plugin>/unload").put(node_plugin_unload)),
        )
        .push(
            Router::with_path("stats")
                .get(get_stats)
                .push(Router::with_path("sum").get(get_stats_sum))
                .push(Router::with_path("<id>").get(get_stats)),
        )
        .push(
            Router::with_path("metrics")
                .get(get_metrics)
                .push(Router::with_path("sum").get(get_metrics_sum))
                .push(Router::with_path("<id>").get(get_metrics)),
        )
}

pub(crate) async fn listen_and_serve(
    laddr: SocketAddr,
    cfg: PluginConfigType,
    rx: oneshot::Receiver<()>,
) -> Result<()> {
    let (reuseaddr, reuseport) = {
        let cfg = cfg.read().await;
        (cfg.http_reuseaddr, cfg.http_reuseport)
    };
    log::info!("HTTP API Listening on {}, reuseaddr: {}, reuseport: {}", laddr, reuseaddr, reuseport);

    let listen = rmqtt::tokio::net::TcpListener::from_std(rmqtt::grpc::server::Server::bind(
        laddr, 128, reuseaddr, reuseport,
    )?)?;
    let incoming = AddrIncoming::from_listener(listen).map_err(anyhow::Error::new)?;
    Server::new(TcpListener::bind(incoming))
        .try_serve_with_graceful_shutdown(route(cfg), async {
            rx.await.ok();
        })
        .await
        .map_err(anyhow::Error::new)?;
    Ok(())
}

#[handler]
async fn list_apis(res: &mut Response) {
    let data = serde_json::json!([
        {
            "name": "get_brokers",
            "method": "GET",
            "path": "/brokers/{node}",
            "descr": "Return the basic information of all nodes in the cluster"
        },
        {
            "name": "get_nodes",
            "method": "GET",
            "path": "/nodes/{node}",
            "descr": "Returns the status of the node"
        },
        {
            "name": "check_health",
            "method": "GET",
            "path": "/health/check",
            "descr": "Node health check"
        },
        {
            "name": "search_clients",
            "method": "GET",
            "path": "/clients/",
            "descr": "Search clients information from the cluster"
        },
        {
            "name": "get_client",
            "method": "GET",
            "path": "/clients/{clientid}",
            "descr": "Get client information from the cluster"
        },
        {
            "name": "kick_client",
            "method": "DELETE",
            "path": "/clients/{clientid}",
            "descr": "Kick client from the cluster"
        },
        {
            "name": "check_online",
            "method": "GET",
            "path": "/clients/{clientid}/online",
            "descr": "Check a client whether online from the cluster"
        },

        {
            "name": "query_subscriptions",
            "method": "GET",
            "path": "/subscriptions",
            "descr": "Query subscriptions information from the cluster"
        },
        {
            "name": "get_client_subscriptions",
            "method": "GET",
            "path": "/subscriptions/{clientid}",
            "descr": "Get subscriptions information for the client from the cluster"
        },

        {
            "name": "get_routes",
            "method": "GET",
            "path": "/routes",
            "descr": "Return all routing information from the cluster"
        },
        {
            "name": "get_route",
            "method": "GET",
            "path": "/routes/{topic}",
            "descr": "Get routing information from the cluster"
        },

        {
            "name": "publish",
            "method": "POST",
            "path": "/mqtt/publish",
            "descr": "Publish MQTT message"
        },
        {
            "name": "subscribe",
            "method": "POST",
            "path": "/mqtt/subscribe",
            "descr": "Subscribe to MQTT topic"
        },
        {
            "name": "unsubscribe",
            "method": "POST",
            "path": "/mqtt/unsubscribe",
            "descr": "Unsubscribe"
        },

        {
            "name": "all_plugins",
            "method": "GET",
            "path": "/plugins/",
            "descr": "Returns information of all plugins in the cluster"
        },
        {
            "name": "node_plugins",
            "method": "GET",
            "path": "/plugins/{node}",
            "descr": "Similar with GET /api/v1/plugins, return the plugin information under the specified node"
        },
        {
            "name": "node_plugin_info",
            "method": "GET",
            "path": "/plugins/{node}/{plugin}",
            "descr": "Get a plugin info"
        },
        {
            "name": "node_plugin_config",
            "method": "GET",
            "path": "/plugins/{node}/{plugin}/config",
            "descr": "Get a plugin config"
        },
        {
            "name": "node_plugin_config_reload",
            "method": "PUT",
            "path": "/plugins/{node}/{plugin}/config/reload",
            "descr": "Reload a plugin config"
        },
        {
            "name": "node_plugin_load",
            "method": "PUT",
            "path": "/plugins/{node}/{plugin}/load",
            "descr": "Load the specified plugin under the specified node."
        },
        {
            "name": "node_plugin_unload",
            "method": "PUT",
            "path": "/plugins/{node}/{plugin}/unload",
            "descr": "Unload the specified plugin under the specified node."
        },

        {
            "name": "get_stats",
            "method": "GET",
            "path": "/stats/{node}",
            "descr": "Returns all statistics information from the cluster"
        },
        {
            "name": "get_stats_sum",
            "method": "GET",
            "path": "/stats/sum",
            "descr": "Summarize all statistics information from the cluster"
        },

        {
            "name": "get_metrics",
            "method": "GET",
            "path": "/metrics/{node}",
            "descr": "Returns all metrics information from the cluster"
        },
        {
            "name": "get_metrics_sum",
            "method": "GET",
            "path": "/metrics/sum",
            "descr": "Summarize all metrics information from the cluster"
        },

    ]);
    res.render(Json(data));
}

#[handler]
async fn api_logger(req: &mut Request, depot: &mut Depot) {
    if let Some(cfg) = depot.obtain::<PluginConfigType>() {
        if !cfg.read().await.http_request_log {
            return;
        }
    }

    let log_data = format!(
        "Request {}, {:?}, {}, {}",
        req.remote_addr().map(|addr| addr.to_string()).unwrap_or_else(|| "[Unknown]".into()),
        req.version(),
        req.method(),
        req.uri()
    );
    let txt_body = if let Some(m) = req.content_type() {
        if let mime::PLAIN | mime::JSON | mime::TEXT = m.subtype() {
            if let Ok(body) = req.payload().await {
                Some(String::from_utf8_lossy(body))
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    if let Some(txt_body) = txt_body {
        log::info!("{}, body: {}", log_data, txt_body);
    } else {
        log::info!("{}", log_data);
    }
}

#[handler]
async fn get_brokers(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_broker(message_type, id).await {
            Ok(Some(broker_info)) => res.render(Json(broker_info)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        match _get_brokers(message_type).await {
            Ok(brokers) => res.render(Json(brokers)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _get_broker(message_type: MessageType, id: NodeId) -> Result<Option<serde_json::Value>> {
    if id == Runtime::instance().node.id() {
        Ok(Some(Runtime::instance().node.broker_info().await.to_json()))
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let msg = Message::BrokerInfo.encode()?;
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(msg)).send().await;
            let broker_info = match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::BrokerInfo(broker_info) => broker_info.to_json(),
                    _ => unreachable!(),
                },
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get GrpcMessage::BrokerInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Ok(Some(broker_info))
        } else {
            Ok(None)
        }
    }
}

#[inline]
async fn _get_brokers(message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut brokers = vec![Runtime::instance().node.broker_info().await.to_json()];
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::BrokerInfo.encode()?;
        let replys = MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await
            .drain(..)
            .map(|reply| match reply {
                (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg) {
                    Ok(MessageReply::BrokerInfo(broker_info)) => Ok(broker_info.to_json()),
                    Err(e) => Err(e),
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::BrokerInfo from other node({}), error: {:?}", id, e);
                    Ok(serde_json::Value::String(e.to_string()))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        brokers.extend(replys);
    }
    Ok(brokers)
}

#[handler]
async fn get_nodes(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_node(message_type, id).await {
            Ok(Some(node_info)) => res.render(Json(node_info)),
            Ok(None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        match _get_nodes(message_type).await {
            Ok(node_infos) => res.render(Json(node_infos)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _get_node(message_type: MessageType, id: NodeId) -> Result<Option<serde_json::Value>> {
    if id == Runtime::instance().node.id() {
        Ok(Some(Runtime::instance().node.node_info().await.to_json()))
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let msg = Message::NodeInfo.encode()?;
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(msg)).send().await;
            let node_info = match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::NodeInfo(node_info) => node_info.to_json(),
                    _ => unreachable!(),
                },
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get GrpcMessage::NodeInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Ok(Some(node_info))
        } else {
            Ok(None)
        }
    }
}

#[inline]
async fn _get_nodes(message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut nodes = vec![Runtime::instance().node.node_info().await.to_json()];
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::NodeInfo.encode()?;
        let replys = MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await
            .drain(..)
            .map(|reply| match reply {
                (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg) {
                    Ok(MessageReply::NodeInfo(node_info)) => Ok(node_info.to_json()),
                    Err(e) => Err(e),
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::NodeInfo from other node({}), error: {:?}", id, e);
                    Ok(serde_json::Value::String(e.to_string()))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        nodes.extend(replys);
    }
    Ok(nodes)
}

#[handler]
async fn check_health(_req: &mut Request, _depot: &mut Depot, res: &mut Response) {
    match Runtime::instance().extends.shared().await.check_health().await {
        Ok(Some(health_info)) => res.render(Json(health_info)),
        Ok(None) => res.set_status_code(StatusCode::NOT_FOUND),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

#[handler]
async fn get_client(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        match _get_client(message_type, &clientid).await {
            Ok(Some(reply)) => res.render(Json(reply)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        res.set_status_error(StatusError::bad_request())
    }
}

async fn _get_client(message_type: MessageType, clientid: &str) -> Result<Option<serde_json::Value>> {
    let reply = clients::get(clientid).await;
    if let Some(reply) = reply {
        return Ok(Some(reply.to_json()));
    }

    let check_result = |reply: GrpcMessageReply| match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res) {
            Ok(MessageReply::ClientGet(ress)) => match ress {
                Some(res) => Ok(res),
                None => Err(MqttError::None),
            },
            Err(e) => Err(e),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    };

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let q = Message::ClientGet { clientid }.encode()?;
        let reply = MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(q))
            .select_ok(check_result)
            .await?;
        return Ok(Some(reply.to_json()));
    }

    Ok(None)
}

#[handler]
async fn search_clients(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<ClientSearchParams>() {
        Ok(q) => q,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string())),
    };

    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    match _search_clients(message_type, q).await {
        Ok(replys) => res.render(Json(replys)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _search_clients(
    message_type: MessageType,
    mut q: ClientSearchParams,
) -> Result<Vec<serde_json::Value>> {
    let mut replys = clients::search(&q).await;
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    for (_id, (_addr, c)) in grpc_clients.iter() {
        if replys.len() < q._limit {
            q._limit -= replys.len();

            let q = Message::ClientSearch(Box::new(q.clone())).encode()?;
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(q)).send().await;
            match reply {
                Ok(GrpcMessageReply::Data(res)) => match MessageReply::decode(&res)? {
                    MessageReply::ClientSearch(ress) => {
                        replys.extend(ress);
                    }
                    _ => unreachable!(),
                },
                Err(e) => {
                    log::warn!("_search_clients, error: {:?}", e);
                }
                _ => unreachable!(),
            };
        } else {
            break;
        }
    }

    let replys = replys.iter().map(|res| res.to_json()).collect::<Vec<_>>();
    Ok(replys)
}

#[handler]
async fn kick_client(req: &mut Request, res: &mut Response) {
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let mut entry = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(Id::from(Runtime::instance().node.id(), ClientId::from(clientid)));

        match entry.kick(true, true, true).await {
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
            Ok(None) => res.set_status_code(StatusCode::NOT_FOUND),
            Ok(Some(offline_info)) => res.render(Text::Plain(offline_info.id.to_string())),
        }
    } else {
        res.set_status_error(StatusError::bad_request())
    }
}

#[handler]
async fn check_online(req: &mut Request, res: &mut Response) {
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let entry = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(Id::from(Runtime::instance().node.id(), ClientId::from(clientid)));

        let online = entry.online().await;
        res.render(Json(online));
    } else {
        res.set_status_error(StatusError::bad_request())
    }
}

#[handler]
async fn query_subscriptions(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<SubsSearchParams>() {
        Ok(q) => q,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string())),
    };
    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    let replys = Runtime::instance().extends.shared().await.query_subscriptions(q).await;
    res.render(Json(replys));
}

#[handler]
async fn get_client_subscriptions(req: &mut Request, res: &mut Response) {
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let entry = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(Id::from(Runtime::instance().node.id(), ClientId::from(clientid)));
        if let Some(subs) = entry.subscriptions().await {
            res.render(Json(subs));
        } else {
            res.set_status_code(StatusCode::NOT_FOUND)
        }
    } else {
        res.set_status_error(StatusError::bad_request())
    }
}

#[handler]
async fn get_routes(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let max_row_limit = cfg.read().await.max_row_limit;
    let limit = req.query::<usize>("_limit");
    let limit = if let Some(limit) = limit {
        if limit > max_row_limit {
            max_row_limit
        } else {
            limit
        }
    } else {
        max_row_limit
    };
    let replys = Runtime::instance().extends.router().await.gets(limit).await;
    res.render(Json(replys));
}

#[handler]
async fn get_route(req: &mut Request, res: &mut Response) {
    let topic = req.param::<String>("topic");
    if let Some(topic) = topic {
        match Runtime::instance().extends.router().await.get(&topic).await {
            Ok(replys) => res.render(Json(replys)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        res.set_status_error(StatusError::bad_request())
    }
}

#[handler]
async fn publish(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let http_laddr = cfg.read().await.http_laddr;

    let remote_addr = req.remote_addr().and_then(|addr| {
        if let Some(ipv4) = addr.as_ipv4() {
            Some(SocketAddr::V4(*ipv4))
        } else {
            addr.as_ipv6().map(|ipv6| SocketAddr::V6(*ipv6))
        }
    });

    let params = match req.parse_json::<PublishParams>().await {
        Ok(p) => p,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string())),
    };
    match _publish(params, remote_addr, http_laddr).await {
        Ok(()) => res.render(Text::Plain("ok")),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _publish(
    params: PublishParams,
    remote_addr: Option<SocketAddr>,
    http_laddr: SocketAddr,
) -> Result<()> {
    let mut topics = if let Some(topics) = params.topics {
        topics.split(',').collect::<Vec<_>>().iter().map(|t| TopicName::from(t.trim())).collect()
    } else {
        Vec::new()
    };
    if let Some(topic) = params.topic {
        topics.push(topic);
    }
    if topics.is_empty() {
        return Err(MqttError::Msg("topics or topic is empty".into()));
    }
    let qos = QoS::try_from(params.qos).map_err(|e| anyhow::Error::msg(e.to_string()))?;
    let encoding = params.encoding.to_ascii_lowercase();
    let payload = if encoding == "plain" {
        bytes::Bytes::from(params.payload)
    } else if encoding == "base64" {
        bytes::Bytes::from(general_purpose::STANDARD.decode(params.payload).map_err(anyhow::Error::new)?)
    } else {
        return Err(MqttError::Msg("encoding error, currently only plain and base64 are supported".into()));
    };

    let from = From::from_admin(Id::new(
        Runtime::instance().node.id(),
        Some(http_laddr),
        remote_addr,
        params.clientid,
        Some(UserName::from("admin")),
    ));
    let p = Publish {
        dup: false,
        retain: params.retain,
        qos,
        topic: "".into(),
        packet_id: None,
        payload,
        properties: PublishProperties::default(),
        create_time: chrono::Local::now().timestamp_millis(),
    };

    let mut futs = Vec::new();
    for topic in topics {
        let mut p1 = p.clone();
        p1.topic = topic;

        //self.listen_cfg.retain_available &&
        if p1.retain() {
            Runtime::instance()
                .extends
                .retain()
                .await
                .set(p1.topic(), Retain { from: from.clone(), publish: p1.clone() })
                .await?;
        }

        let fut = async {
            //hook, message_publish
            let p1 = Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_publish(None, None, from.clone(), &p1)
                .await
                .unwrap_or(p1);

            let replys = Runtime::instance().extends.shared().await.forwards(from.clone(), p1).await;
            match replys {
                Ok(0) => {
                    //hook, message_nonsubscribed
                    Runtime::instance().extends.hook_mgr().await.message_nonsubscribed(from.clone()).await;
                }
                Ok(_) => {}
                Err(droppeds) => {
                    for (to, from, p, reason) in droppeds {
                        //Message dropped
                        Runtime::instance()
                            .extends
                            .hook_mgr()
                            .await
                            .message_dropped(Some(to), from, p, reason)
                            .await;
                    }
                }
            }
        };
        futs.push(fut);
    }
    let _ = futures::future::join_all(futs).await;
    Ok(())
}

#[handler]
async fn subscribe(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let params = match req.parse_json::<SubscribeParams>().await {
        Ok(p) => p,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string())),
    };

    let node_id = if let Some(status) =
        Runtime::instance().extends.shared().await.session_status(&params.clientid).await
    {
        if status.online {
            status.id.node_id
        } else {
            res.set_status_error(StatusError::service_unavailable().with_detail("the session is offline"));
            return;
        }
    } else {
        res.set_status_error(StatusError::not_found().with_detail("session does not exist"));
        return;
    };

    if node_id == Runtime::instance().node.id() {
        #[allow(clippy::mutable_key_type)]
        match subs::subscribe(params).await {
            Ok(replys) => {
                let replys = replys
                    .into_iter()
                    .map(|(t, r)| {
                        let r = match r {
                            Ok(b) => serde_json::Value::Bool(b),
                            Err(e) => serde_json::Value::String(e.to_string()),
                        };
                        (t, r)
                    })
                    .collect::<HashMap<_, _>>();
                res.render(Json(replys))
            }
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
        let message_type = cfg.read().await.message_type;
        //The session is on another node
        #[allow(clippy::mutable_key_type)]
        match _subscribe_on_other_node(message_type, node_id, params).await {
            Ok(replys) => {
                let replys = replys
                    .into_iter()
                    .map(|(t, r)| {
                        let r = match r {
                            (b, None) => serde_json::Value::Bool(b),
                            (true, _) => serde_json::Value::Bool(true),
                            (false, Some(reason)) => serde_json::Value::String(reason),
                        };
                        (t, r)
                    })
                    .collect::<HashMap<_, _>>();
                res.render(Json(replys))
            }
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _subscribe_on_other_node(
    message_type: MessageType,
    node_id: NodeId,
    params: SubscribeParams,
) -> Result<HashMap<TopicFilter, (bool, Option<String>)>> {
    let c = get_grpc_client(node_id).await?;
    let q = Message::Subscribe(params).encode()?;
    let reply = MessageSender::new(c, message_type, GrpcMessage::Data(q)).send().await?;
    match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res)? {
            MessageReply::Subscribe(ress) => Ok(ress),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

#[handler]
async fn unsubscribe(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let params = match req.parse_json::<UnsubscribeParams>().await {
        Ok(p) => p,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string())),
    };

    let node_id = if let Some(status) =
        Runtime::instance().extends.shared().await.session_status(&params.clientid).await
    {
        if status.online {
            status.id.node_id
        } else {
            res.set_status_error(StatusError::service_unavailable().with_detail("the session is offline"));
            return;
        }
    } else {
        res.set_status_error(StatusError::not_found().with_detail("session does not exist"));
        return;
    };

    if node_id == Runtime::instance().node.id() {
        match subs::unsubscribe(params).await {
            Ok(()) => res.render(Json(true)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
        let message_type = cfg.read().await.message_type;
        //The session is on another node
        match _unsubscribe_on_other_node(message_type, node_id, params).await {
            Ok(()) => res.render(Text::Plain("ok")),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _unsubscribe_on_other_node(
    message_type: MessageType,
    node_id: NodeId,
    params: UnsubscribeParams,
) -> Result<()> {
    let c = get_grpc_client(node_id).await?;
    let q = Message::Unsubscribe(params).encode()?;
    let reply = MessageSender::new(c, message_type, GrpcMessage::Data(q)).send().await?;
    match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res)? {
            MessageReply::Unsubscribe => Ok(()),
            _ => unreachable!(),
        },
        _ => unreachable!(),
    }
}

#[handler]
async fn all_plugins(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    match _all_plugins(message_type).await {
        Ok(pluginss) => res.render(Json(pluginss)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

#[inline]
async fn _all_plugins(message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut pluginss = Vec::new();
    let node_id = Runtime::instance().node.id();
    let plugins = plugin::get_plugins().await?;
    let plugins = plugins.into_iter().map(|p| p.to_json()).collect::<Result<Vec<_>>>()?;
    pluginss.push(json!({
        "node": node_id,
        "plugins": plugins,
    }));

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::GetPlugins.encode()?;
        let replys = MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await
            .drain(..)
            .map(|(node_id, reply)| {
                let plugins = match reply {
                    Ok(GrpcMessageReply::Data(reply_msg)) => match MessageReply::decode(&reply_msg) {
                        Ok(MessageReply::GetPlugins(plugins)) => {
                            match plugins.into_iter().map(|p| p.to_json()).collect::<Result<Vec<_>>>() {
                                Ok(plugins) => serde_json::Value::Array(plugins),
                                Err(e) => serde_json::Value::String(e.to_string()),
                            }
                        }
                        Err(e) => serde_json::Value::String(e.to_string()),
                        _ => unreachable!(),
                    },
                    Ok(_) => unreachable!(),
                    Err(e) => serde_json::Value::String(e.to_string()),
                };
                json!({
                    "node": node_id,
                    "plugins": plugins,
                })
            })
            .collect::<Vec<_>>();
        pluginss.extend(replys);
    }
    Ok(pluginss)
}

#[handler]
async fn node_plugins(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    match _node_plugins(node_id, message_type).await {
        Ok(plugins) => res.render(Json(plugins)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugins(node_id: NodeId, message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let plugins = if node_id == Runtime::instance().node.id() {
        plugin::get_plugins().await?
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::GetPlugins.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPlugins(plugins) => plugins,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    };
    plugins.into_iter().map(|p| p.to_json()).collect::<Result<Vec<_>>>()
}

#[handler]
async fn node_plugin_info(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };

    match _node_plugin_info(node_id, &name, message_type).await {
        Ok(plugin) => res.render(Json(plugin)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugin_info(
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<Option<serde_json::Value>> {
    let plugin = if node_id == Runtime::instance().node.id() {
        plugin::get_plugin(name).await?
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::GetPlugin { name }.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPlugin(plugin) => plugin,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    };
    if let Some(plugin) = plugin {
        Ok(Some(plugin.to_json()?))
    } else {
        Ok(None)
    }
}

#[handler]
async fn node_plugin_config(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };

    match _node_plugin_config(node_id, &name, message_type).await {
        Ok(cfg) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
            res.write_body(cfg).ok();
        }
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugin_config(node_id: NodeId, name: &str, message_type: MessageType) -> Result<Vec<u8>> {
    let plugin_cfg = if node_id == Runtime::instance().node.id() {
        plugin::get_plugin_config(name).await?
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::GetPluginConfig { name }.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPluginConfig(cfg) => cfg,
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    };
    Ok(plugin_cfg)
}

#[handler]
async fn node_plugin_config_reload(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };

    match _node_plugin_config_reload(node_id, &name, message_type).await {
        Ok(()) => res.render(Text::Plain("ok")),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugin_config_reload(node_id: NodeId, name: &str, message_type: MessageType) -> Result<()> {
    if node_id == Runtime::instance().node.id() {
        Runtime::instance().plugins.load_config(name).await
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::ReloadPluginConfig { name }.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::ReloadPluginConfig => Ok(()),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}

#[handler]
async fn node_plugin_load(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };

    match _node_plugin_load(node_id, &name, message_type).await {
        Ok(()) => res.render(Text::Plain("ok")),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugin_load(node_id: NodeId, name: &str, message_type: MessageType) -> Result<()> {
    if node_id == Runtime::instance().node.id() {
        Runtime::instance().plugins.start(name).await
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::LoadPlugin { name }.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::LoadPlugin => Ok(()),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}

#[handler]
async fn node_plugin_unload(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        return res.set_status_code(StatusCode::NOT_FOUND);
    };

    match _node_plugin_unload(node_id, &name, message_type).await {
        Ok(r) => res.render(Json(r)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _node_plugin_unload(node_id: NodeId, name: &str, message_type: MessageType) -> Result<bool> {
    if node_id == Runtime::instance().node.id() {
        Runtime::instance().plugins.stop(name).await
    } else {
        let c = get_grpc_client(node_id).await?;
        let msg = Message::UnloadPlugin { name }.encode()?;
        let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::UnloadPlugin(ok) => Ok(ok),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        }
    }
}

#[handler]
async fn get_stats_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    match _get_stats_sum(message_type).await {
        Ok(stats_sum) => res.render(Json(stats_sum)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _get_stats_sum(message_type: MessageType) -> Result<serde_json::Value> {
    let this_id = Runtime::instance().node.id();
    let mut nodes = HashMap::default();
    nodes.insert(
        this_id,
        json!({
            "name": Runtime::instance().node.name(this_id).await,
            "status": Runtime::instance().node.status().await,
        }),
    );

    let mut stats_sum = Runtime::instance().stats.clone().await;
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg)).join_all().await
        {
            match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => {
                        nodes.insert(
                            id,
                            json!({
                                "name": Runtime::instance().node.name(id).await,
                                "status": node_status,
                            }),
                        );
                        stats_sum.add(*stats);
                    }
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node({}), error: {:?}", id, e);
                    nodes.insert(id, serde_json::Value::String(e.to_string()));
                }
            };
        }
    }

    let stats_sum = json!({
        "nodes": nodes,
        "stats": stats_sum.to_json().await
    });

    Ok(stats_sum)
}

#[handler]
async fn get_stats(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_stats_one(message_type, id).await {
            Ok(Some(stat_info)) => res.render(Json(stat_info)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        match _get_stats_all(message_type).await {
            Ok(stat_infos) => res.render(Json(stat_infos)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _get_stats_one(message_type: MessageType, id: NodeId) -> Result<Option<serde_json::Value>> {
    if id == Runtime::instance().node.id() {
        let node_status = Runtime::instance().node.status().await;
        let stats = Runtime::instance().stats.clone().await;
        Ok(Some(_build_stats(id, node_status, stats.to_json().await).await))
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some(c) = grpc_clients.get(&id).map(|(_, c)| c.clone()) {
            let msg = Message::StatsInfo.encode()?;
            let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await;
            let stats = match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => {
                        _build_stats(id, node_status, stats.to_json().await).await
                    }
                    _ => unreachable!(),
                },
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Ok(Some(stats))
        } else {
            Ok(None)
        }
    }
}

#[inline]
async fn _get_stats_all(message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let id = Runtime::instance().node.id();
    let node_status = Runtime::instance().node.status().await;
    let state = Runtime::instance().stats.clone().await;
    let mut stats = vec![_build_stats(id, node_status, state.to_json().await).await];

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg)).join_all().await
        {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => {
                        _build_stats(id, node_status, stats.to_json().await).await
                    }
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node({}), error: {:?}", id, e);
                    serde_json::Value::String(e.to_string())
                }
            };
            stats.push(data);
        }
    }
    Ok(stats)
}

#[inline]
async fn _build_stats(id: NodeId, node_status: NodeStatus, stats: serde_json::Value) -> serde_json::Value {
    let node_name = Runtime::instance().node.name(id).await;
    let data = json!({
        "node": {
            "id": id,
            "name": node_name,
            "status": node_status,
        },
        "stats": stats
    });
    data
}

#[handler]
async fn get_metrics(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_metrics_one(message_type, id).await {
            Ok(Some(metrics)) => res.render(Json(metrics)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    } else {
        match _get_metrics_all(message_type).await {
            Ok(metricses) => res.render(Json(metricses)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
        }
    }
}

#[inline]
async fn _get_metrics_one(message_type: MessageType, id: NodeId) -> Result<Option<serde_json::Value>> {
    if id == Runtime::instance().node.id() {
        let metrics = Runtime::instance().metrics.to_json();
        Ok(Some(_build_metrics(id, metrics).await))
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some(c) = grpc_clients.get(&id).map(|(_, c)| c.clone()) {
            let msg = Message::MetricsInfo.encode()?;
            let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg)).send().await;
            let metrics = match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => _build_metrics(id, metrics.to_json()).await,
                    _ => unreachable!(),
                },
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Ok(Some(metrics))
        } else {
            Ok(None)
        }
    }
}

#[inline]
async fn _get_metrics_all(message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let id = Runtime::instance().node.id();
    let metrics = Runtime::instance().metrics.to_json();
    let mut metricses = vec![_build_metrics(id, metrics).await];

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::MetricsInfo.encode()?;
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg)).join_all().await;
        for reply in replys {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => _build_metrics(id, metrics.to_json()).await,
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node({}), error: {:?}", id, e);
                    serde_json::Value::String(e.to_string())
                }
            };
            metricses.push(data);
        }
    }
    Ok(metricses)
}

#[handler]
async fn get_metrics_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().await.message_type;

    match _get_metrics_sum(message_type).await {
        Ok(metrics_sum) => res.render(Json(metrics_sum)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string())),
    }
}

async fn _get_metrics_sum(message_type: MessageType) -> Result<serde_json::Value> {
    let mut metrics_sum = Runtime::instance().metrics.clone();
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::MetricsInfo.encode()?;
        for reply in
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg)).join_all().await
        {
            match reply {
                (_id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => metrics_sum.add(&metrics),
                    _ => unreachable!(),
                },
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node({}), error: {:?}", id, e);
                }
            };
        }
    }

    Ok(metrics_sum.to_json())
}

#[inline]
async fn _build_metrics(id: NodeId, metrics: serde_json::Value) -> serde_json::Value {
    let node_name = Runtime::instance().node.name(id).await;
    let data = json!({
        "node": {
            "id": id,
            "name": node_name,
        },
        "metrics": metrics
    });
    data
}

#[inline]
async fn get_grpc_client(node_id: NodeId) -> Result<NodeGrpcClient> {
    Runtime::instance()
        .extends
        .shared()
        .await
        .get_grpc_clients()
        .get(&node_id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| MqttError::from("node grpc client is not exist!"))
}
