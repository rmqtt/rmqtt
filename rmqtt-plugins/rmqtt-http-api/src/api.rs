use std::convert::From as _;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use salvo::conn::tcp::TcpAcceptor;
use salvo::http::header::{HeaderValue, CONTENT_TYPE};
use salvo::http::mime;
use salvo::prelude::*;

use anyhow::anyhow;
use base64::prelude::{Engine, BASE64_STANDARD};
use serde_json::{self, json};
use tokio::sync::oneshot;

use rmqtt::{
    codec::types::Publish,
    codec::v5::PublishProperties,
    context::ServerContext,
    grpc::{
        GrpcClient, Message as GrpcMessage, MessageBroadcaster, MessageReply as GrpcMessageReply,
        MessageSender, MessageType,
    },
    metrics::Metrics,
    net::MqttError,
    node::{NodeInfo, NodeStatus},
    session::SessionState,
    stats::Stats,
    types::NodeId,
    types::{ClientId, From, HashMap, Id, QoS, SubsSearchParams, TopicFilter, TopicName, UserName},
    utils::timestamp_millis,
    Result,
};

use super::types::{
    ClientSearchParams, ClientSearchResult, Message, MessageReply, PrometheusDataType, PublishParams,
    SubscribeParams, UnsubscribeParams,
};
use super::{clients, plugin, prome, subs, PluginConfigType};

struct BearerValidator {
    token: String,
}
impl BearerValidator {
    pub fn new(token: &str) -> Self {
        Self { token: format!("Bearer {token}") }
    }
}

#[async_trait]
impl Handler for BearerValidator {
    async fn handle(&self, req: &mut Request, depot: &mut Depot, res: &mut Response, ctrl: &mut FlowCtrl) {
        if req.headers().get("authorization").is_some_and(|token| token == &self.token) {
            ctrl.call_next(req, depot, res).await;
        } else {
            res.status_code(StatusCode::UNAUTHORIZED);
            ctrl.skip_rest()
        }
    }
}

fn route(scx: ServerContext, cfg: PluginConfigType, token: Option<String>) -> Router {
    let mut router = Router::with_path("api/v1").hoop(affix_state::inject((scx, cfg))).hoop(api_logger);
    if let Some(token) = token {
        router = router.hoop(BearerValidator::new(&token));
    }
    router
        .get(list_apis)
        .push(Router::with_path("brokers").get(get_brokers).push(Router::with_path("{id}").get(get_brokers)))
        .push(Router::with_path("nodes").get(get_nodes).push(Router::with_path("{id}").get(get_nodes)))
        .push(Router::with_path("health/check").get(check_health))
        .push(
            Router::with_path("clients")
                .push(Router::with_path("offlines").get(search_offlines).delete(kick_offlines))
                .get(search_clients)
                .push(
                    Router::with_path("{clientid}")
                        .get(get_client)
                        .delete(kick_client)
                        .push(Router::with_path("online").get(check_online)),
                ),
        )
        .push(
            Router::with_path("subscriptions")
                .get(query_subscriptions)
                .push(Router::with_path("{clientid}").get(get_client_subscriptions)),
        )
        .push(Router::with_path("routes").get(get_routes).push(Router::with_path("{topic}").get(get_route)))
        .push(
            Router::with_path("mqtt")
                .push(Router::with_path("publish").post(publish))
                .push(Router::with_path("subscribe").post(subscribe))
                .push(Router::with_path("unsubscribe").post(unsubscribe)),
        )
        .push(
            Router::with_path("plugins")
                .get(all_plugins)
                .push(Router::with_path("{node}").get(node_plugins))
                .push(Router::with_path("{node}/{plugin}").get(node_plugin_info))
                .push(Router::with_path("{node}/{plugin}/config").get(node_plugin_config))
                .push(Router::with_path("{node}/{plugin}/config/reload").put(node_plugin_config_reload))
                .push(Router::with_path("{node}/{plugin}/load").put(node_plugin_load))
                .push(Router::with_path("{node}/{plugin}/unload").put(node_plugin_unload)),
        )
        .push(
            Router::with_path("stats")
                .get(get_stats)
                .push(Router::with_path("sum").get(get_stats_sum))
                .push(Router::with_path("{id}").get(get_stats)),
        )
        .push(
            Router::with_path("metrics")
                .get(get_metrics)
                .push(
                    Router::with_path("prometheus")
                        .get(get_prometheus_metrics)
                        .push(Router::with_path("sum").get(get_prometheus_metrics_sum))
                        .push(Router::with_path("{id}").get(get_prometheus_metrics)),
                )
                .push(Router::with_path("sum").get(get_metrics_sum))
                .push(Router::with_path("{id}").get(get_metrics)),
        )
}

pub(crate) async fn listen_and_serve(
    scx: ServerContext,
    laddr: SocketAddr,
    cfg: PluginConfigType,
    rx: oneshot::Receiver<()>,
) -> Result<()> {
    let (reuseaddr, reuseport, http_bearer_token) = {
        let cfg = cfg.read().await;
        (cfg.http_reuseaddr, cfg.http_reuseport, cfg.http_bearer_token.clone())
    };
    log::info!("HTTP API Listening on {}, reuseaddr: {}, reuseport: {}", laddr, reuseaddr, reuseport);

    let listen = tokio::net::TcpListener::from_std(bind(laddr, 128, reuseaddr, reuseport)?)?;

    let acceptor = TcpAcceptor::try_from(listen)?;
    let server = Server::new(acceptor);
    let handler = server.handle();
    tokio::task::spawn(async move {
        rx.await.ok();
        handler.stop_graceful(None);
    });
    server.try_serve(route(scx, cfg, http_bearer_token)).await?;
    Ok(())
}

#[inline]
fn bind(
    laddr: std::net::SocketAddr,
    backlog: i32,
    _reuseaddr: bool,
    _reuseport: bool,
) -> Result<std::net::TcpListener> {
    use socket2::{Domain, SockAddr, Socket, Type};
    let builder = Socket::new(Domain::for_address(laddr), Type::STREAM, None)?;
    builder.set_nonblocking(true)?;
    #[cfg(unix)]
    builder.set_reuse_address(_reuseaddr)?;
    #[cfg(unix)]
    builder.set_reuse_port(_reuseport)?;
    builder.bind(&SockAddr::from(laddr))?;
    builder.listen(backlog)?;
    Ok(std::net::TcpListener::from(builder))
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
            "name": "search_offlines",
            "method": "GET",
            "path": "/clients/offlines",
            "descr": "Search offlines clients information from the cluster"
        },
        {
            "name": "kick_offlines",
            "method": "DELETE",
            "path": "/clients/offlines",
            "descr": "Kick offlines clients from the cluster"
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

        {
          "name": "get_prometheus_metrics",
          "method": "GET",
          "path": "/metrics/prometheus",
          "descr": "Get prometheus metrics from the cluster"
        },


    ]);
    res.render(Json(data));
}

fn get_scx_cfg(depot: &mut Depot) -> std::result::Result<&(ServerContext, PluginConfigType), salvo::Error> {
    let scx_cfg = depot.obtain::<(ServerContext, PluginConfigType)>().map_err(|e| match e {
        None => salvo::Error::Io(std::io::Error::new(ErrorKind::NotFound, anyhow!("None"))),
        Some(e) => salvo::Error::Io(std::io::Error::new(ErrorKind::NotFound, format!("{:?}", e))),
    })?;
    Ok(scx_cfg)
}

#[handler]
async fn api_logger(req: &mut Request, depot: &mut Depot) -> std::result::Result<(), salvo::Error> {
    let (_, cfg) = get_scx_cfg(depot)?;
    if !cfg.read().await.http_request_log {
        return Ok(());
    }
    let log_data =
        format!("Request {}, {:?}, {}, {}", req.remote_addr(), req.version(), req.method(), req.uri());
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
    Ok(())
}

#[handler]
async fn get_brokers(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_broker(scx, message_type, id).await {
            Ok(Some(broker_info)) => res.render(Json(broker_info)),
            Ok(None) => {
                //| Err(MqttError::None)
                res.status_code(StatusCode::NOT_FOUND);
            }
            Err(e) => {
                res.render(StatusError::service_unavailable().detail(e.to_string()));
            }
        }
    } else {
        match _get_brokers(scx, message_type).await {
            Ok(brokers) => res.render(Json(brokers)),
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
async fn _get_broker(
    scx: &ServerContext,
    message_type: MessageType,
    id: NodeId,
) -> Result<Option<serde_json::Value>> {
    if id == scx.node.id() {
        Ok(Some(scx.node.broker_info(scx).await.to_json()))
    } else {
        let grpc_clients = scx.extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let msg = Message::BrokerInfo.encode()?;
            let reply = MessageSender::new(
                c.clone(),
                message_type,
                GrpcMessage::Data(msg),
                Some(Duration::from_secs(10)),
            )
            .send()
            .await;
            let broker_info = match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::BrokerInfo(broker_info) => broker_info.to_json(),
                    _ => unreachable!(),
                },
                Ok(reply) => {
                    log::info!("Get GrpcMessage::BrokerInfo from other node({}), reply: {:?}", id, reply);
                    serde_json::Value::String("Invalid Result".into())
                }
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
async fn _get_brokers(scx: &ServerContext, message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut brokers = vec![scx.node.broker_info(scx).await.to_json()];
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::BrokerInfo.encode()?;
        let replys = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        .drain(..)
        .map(|reply| match reply {
            (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg) {
                Ok(MessageReply::BrokerInfo(broker_info)) => Ok(broker_info.to_json()),
                Err(e) => Err(e),
                _ => unreachable!(),
            },
            (id, Ok(reply)) => {
                log::info!("Get GrpcMessage::BrokerInfo from other node({}), reply: {:?}", id, reply);
                Ok(serde_json::Value::String("Invalid Result".into()))
            }
            (id, Err(e)) => {
                log::warn!("Get GrpcMessage::BrokerInfo from other node({}), error: {:?}", id, e);
                Ok(serde_json::Value::String(e.to_string()))
            }
        })
        .collect::<Result<Vec<_>>>()?;
        brokers.extend(replys);
    }
    Ok(brokers)
}

#[handler]
async fn get_nodes(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match get_node(scx, message_type, id).await {
            Ok(Some(node_info)) => res.render(Json(node_info.to_json())),
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        match get_nodes_all(scx, message_type).await {
            Ok(node_infos) => {
                let mut nodes = Vec::new();
                for item in node_infos {
                    match item {
                        Ok(node_info) => {
                            nodes.push(node_info.to_json());
                        }
                        Err(e) => {
                            nodes.push(serde_json::Value::String(e.to_string()));
                        }
                    }
                }
                res.render(Json(nodes))
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
async fn _get_nodes(scx: &ServerContext, message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut nodes = vec![scx.node.node_info(scx).await.to_json()];
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::NodeInfo.encode()?;
        let replys = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        .drain(..)
        .map(|reply| match reply {
            (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg) {
                Ok(MessageReply::NodeInfo(node_info)) => Ok(node_info.to_json()),
                Err(e) => Err(e),
                _ => unreachable!(),
            },
            (id, Ok(reply)) => {
                log::info!("Get GrpcMessage::NodeInfo from other node({}), reply: {:?}", id, reply);
                Err(anyhow!("Invalid Result"))
            }
            (id, Err(e)) => {
                log::warn!("Get GrpcMessage::NodeInfo from other node({}), error: {:?}", id, e);
                Ok(serde_json::Value::String(e.to_string()))
            }
        })
        .collect::<Result<Vec<_>>>()?;
        nodes.extend(replys);
    }
    Ok(nodes)
}

#[inline]
pub(crate) async fn get_node(
    scx: &ServerContext,
    message_type: MessageType,
    id: NodeId,
) -> Result<Option<NodeInfo>> {
    if id == scx.node.id() {
        Ok(Some(scx.node.node_info(scx).await))
    } else {
        let grpc_clients = scx.extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let msg = Message::NodeInfo.encode()?;
            let reply = MessageSender::new(
                c.clone(),
                message_type,
                GrpcMessage::Data(msg),
                Some(Duration::from_secs(10)),
            )
            .send()
            .await;
            match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::NodeInfo(node_info) => Ok(Some(node_info)),
                    _ => unreachable!(),
                },
                Ok(reply) => {
                    log::info!("Get GrpcMessage::NodeInfo from other node({}), reply: {:?}", id, reply);
                    Err(anyhow!("Invalid Result"))
                }
                Err(e) => {
                    log::warn!("Get GrpcMessage::NodeInfo from other node, error: {:?}", e);
                    Err(e)
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[inline]
pub(crate) async fn get_nodes_all(
    scx: &ServerContext,
    message_type: MessageType,
) -> Result<Vec<Result<NodeInfo>>> {
    let mut nodes = vec![Ok(scx.node.node_info(scx).await)];
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::NodeInfo.encode()?;
        let replys = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        .drain(..)
        .map(|reply| match reply {
            (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg) {
                Ok(MessageReply::NodeInfo(node_info)) => Ok(Ok(node_info)),
                Err(e) => Err(e),
                _ => unreachable!(),
            },
            (id, Ok(reply)) => {
                log::info!("Get GrpcMessage::NodeInfo from other node({}), reply: {:?}", id, reply);
                Err(anyhow!("Invalid Result"))
            }
            (id, Err(e)) => {
                log::warn!("Get GrpcMessage::NodeInfo from other node({}), error: {:?}", id, e);
                Ok(Err(e))
            }
        })
        .collect::<Result<Vec<_>>>()?;
        nodes.extend(replys);
    }
    Ok(nodes)
}

#[handler]
async fn check_health(
    _req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, _) = get_scx_cfg(depot)?;
    match scx.extends.shared().await.check_health().await {
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        Ok(health_info) => res.render(Json(health_info)),
    }
    Ok(())
}

#[handler]
async fn get_client(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        match _get_client(scx, message_type, &clientid).await {
            Ok(Some(reply)) => res.render(Json(reply)),
            Ok(None) => {
                //| Err(MqttError::None)
                res.status_code(StatusCode::NOT_FOUND);
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        res.render(StatusError::bad_request())
    }
    Ok(())
}

async fn _get_client(
    scx: &ServerContext,
    message_type: MessageType,
    clientid: &str,
) -> Result<Option<serde_json::Value>> {
    let reply = clients::get(scx, clientid).await;
    if let Some(reply) = reply {
        return Ok(Some(reply.to_json()));
    }

    let check_result = |reply: GrpcMessageReply| match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res) {
            Ok(MessageReply::ClientGet(ress)) => match ress {
                Some(res) => Ok(res),
                None => Err(anyhow!(MqttError::None)),
            },
            Err(e) => Err(e),
            _ => unreachable!(),
        },
        reply => {
            log::info!("Subscribe GrpcMessage::ClientGet from other node, reply: {:?}", reply);
            Err(anyhow!("Invalid Result"))
        }
    };

    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let q = Message::ClientGet { clientid }.encode()?;
        let reply = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(q),
            Some(Duration::from_secs(10)),
        )
        .select_ok(check_result)
        .await?;
        return Ok(Some(reply.to_json()));
    }

    Ok(None)
}

#[handler]
async fn search_clients(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<ClientSearchParams>() {
        Ok(q) => q,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };

    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    match _search_clients(scx, message_type, q).await {
        Ok(replys) => {
            let replys = replys.iter().map(|res| res.to_json()).collect::<Vec<_>>();
            res.render(Json(replys))
        }
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

#[handler]
async fn search_offlines(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<ClientSearchParams>() {
        Ok(q) => q,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };
    q.connected = Some(false);

    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    match _search_clients(scx, message_type, q).await {
        Ok(replys) => {
            let replys = replys.iter().map(|res| res.to_json()).collect::<Vec<_>>();
            res.render(Json(replys))
        }
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _search_clients(
    scx: &ServerContext,
    message_type: MessageType,
    mut q: ClientSearchParams,
) -> Result<Vec<ClientSearchResult>> {
    let mut replys = clients::search(scx, &q).await;
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    for (id, (_addr, c)) in grpc_clients.iter() {
        if replys.len() < q._limit {
            q._limit -= replys.len();

            let q = Message::ClientSearch(Box::new(q.clone())).encode()?;
            let reply = MessageSender::new(
                c.clone(),
                message_type,
                GrpcMessage::Data(q),
                Some(Duration::from_secs(10)),
            )
            .send()
            .await;
            match reply {
                Ok(GrpcMessageReply::Data(res)) => match MessageReply::decode(&res)? {
                    MessageReply::ClientSearch(ress) => {
                        replys.extend(ress);
                    }
                    _ => unreachable!(),
                },
                Err(e) => {
                    log::warn!("Get GrpcMessage::ClientSearch, error: {:?}", e);
                }
                Ok(reply) => {
                    log::warn!("Get GrpcMessage::ClientSearch from other node({}), reply: {:?}", id, reply);
                }
            };
        } else {
            break;
        }
    }

    Ok(replys)
}

#[handler]
async fn kick_client(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, _) = get_scx_cfg(depot)?;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let mut entry = scx.extends.shared().await.entry(Id::from(scx.node.id(), ClientId::from(clientid)));
        let s = entry.session();
        if let Some(s) = s {
            match entry.kick(true, true, true).await {
                Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
                Ok(_) => res.render(Json(s.id.to_json())),
            }
        } else {
            res.status_code(StatusCode::NOT_FOUND);
        }
    } else {
        res.render(StatusError::bad_request())
    }
    Ok(())
}

#[handler]
async fn kick_offlines(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<ClientSearchParams>() {
        Ok(q) => q,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };
    q.connected = Some(false);

    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }

    let mut count = 0;
    match _search_clients(scx, message_type, q).await {
        Ok(replys) => {
            for reply in replys.iter() {
                log::debug!("node_id: {}, clientid: {}", reply.node_id, reply.clientid);
                let mut entry = scx
                    .extends
                    .shared()
                    .await
                    .entry(Id::from(reply.node_id, ClientId::from(reply.clientid.clone())));
                let s = entry.session();
                if s.is_some() {
                    match entry.kick(true, true, true).await {
                        Err(e) => {
                            log::warn!("{}", e);
                        }
                        Ok(_) => {
                            count += 1;
                        }
                    }
                } else {
                    log::warn!(
                        "session is not found, node_id: {}, clientid: {}",
                        reply.node_id,
                        reply.clientid
                    );
                }
            }
        }
        Err(e) => {
            log::warn!("{}", e);
        }
    }
    res.render(Json(json!({"count": count})));
    Ok(())
}

#[handler]
async fn check_online(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, _) = get_scx_cfg(depot)?;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let entry = scx.extends.shared().await.entry(Id::from(scx.node.id(), ClientId::from(clientid)));

        let online = entry.online().await;
        res.render(Json(online));
    } else {
        res.render(StatusError::bad_request())
    }
    Ok(())
}

#[handler]
async fn query_subscriptions(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let max_row_limit = cfg.read().await.max_row_limit;
    let mut q = match req.parse_queries::<SubsSearchParams>() {
        Ok(q) => q,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };
    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    let replys = scx
        .extends
        .shared()
        .await
        .query_subscriptions(q)
        .await
        .into_iter()
        .map(|res| res.to_json())
        .collect::<Vec<serde_json::Value>>();
    res.render(Json(replys));
    Ok(())
}

#[handler]
async fn get_client_subscriptions(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, _) = get_scx_cfg(depot)?;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        let entry = scx.extends.shared().await.entry(Id::from(scx.node.id(), ClientId::from(clientid)));
        if let Some(subs) = entry.subscriptions().await {
            let subs = subs.into_iter().map(|res| res.to_json()).collect::<Vec<serde_json::Value>>();
            res.render(Json(subs));
        } else {
            res.status_code(StatusCode::NOT_FOUND);
        }
    } else {
        res.render(StatusError::bad_request());
    }
    Ok(())
}

#[handler]
async fn get_routes(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
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
    let replys = scx.extends.router().await.gets(limit).await;
    res.render(Json(replys));
    Ok(())
}

#[handler]
async fn get_route(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, _) = get_scx_cfg(depot)?;
    let topic = req.param::<String>("topic");
    if let Some(topic) = topic {
        match scx.extends.router().await.get(&topic).await {
            Ok(replys) => res.render(Json(replys)),
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        res.render(StatusError::bad_request())
    }
    Ok(())
}

#[handler]
async fn publish(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let (http_laddr, expiry_interval) = {
        let cfg_rl = cfg.read().await;
        (cfg_rl.http_laddr, cfg_rl.message_expiry_interval)
    };

    let addr = req.remote_addr();
    let remote_addr = if let Some(ipv4) = addr.as_ipv4() {
        Some(SocketAddr::V4(*ipv4))
    } else {
        addr.as_ipv6().map(|ipv6| SocketAddr::V6(*ipv6))
    };

    let params = match req.parse_json::<PublishParams>().await {
        Ok(p) => p,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };
    match _publish(scx, params, remote_addr, http_laddr, expiry_interval).await {
        Ok(()) => res.render(Text::Plain("ok")),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _publish(
    scx: &ServerContext,
    params: PublishParams,
    remote_addr: Option<SocketAddr>,
    http_laddr: SocketAddr,
    expiry_interval: Duration,
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
        return Err(anyhow!("topics or topic is empty"));
    }
    let qos = QoS::try_from(params.qos).map_err(|e| anyhow::Error::msg(e.to_string()))?;
    let encoding = params.encoding.to_ascii_lowercase();
    let payload = if encoding == "plain" {
        bytes::Bytes::from(params.payload)
    } else if encoding == "base64" {
        bytes::Bytes::from(BASE64_STANDARD.decode(params.payload).map_err(anyhow::Error::new)?)
    } else {
        return Err(anyhow!("encoding error, currently only plain and base64 are supported"));
    };

    let from = From::from_admin(Id::new(
        scx.node.id(),
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
        properties: Some(PublishProperties::default()),
        delay_interval: None,
        create_time: Some(timestamp_millis()),
    };

    let message_expiry_interval = params
        .properties
        .as_ref()
        .and_then(|props| {
            props.message_expiry_interval.map(|interval| Duration::from_secs(interval.get() as u64))
        })
        .unwrap_or(expiry_interval);
    log::debug!("message_expiry_interval: {:?}", message_expiry_interval);

    let storage_available = scx.extends.message_mgr().await.enable();

    let mut futs = Vec::new();
    for topic in topics {
        let from = from.clone();
        let mut p1 = p.clone();
        p1.topic = topic;
        let p1 = Box::new(p1);
        let fut = async move {
            //hook, message_publish
            let p1 = scx.extends.hook_mgr().message_publish(None, from.clone(), &p1).await.unwrap_or(p1);

            if let Err(e) =
                SessionState::forwards(scx, from, p1, storage_available, Some(message_expiry_interval)).await
            {
                log::warn!("{:?}", e);
            }
        };
        futs.push(fut);
    }
    let _ = futures::future::join_all(futs).await;
    Ok(())
}

#[handler]
async fn subscribe(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let params = match req.parse_json::<SubscribeParams>().await {
        Ok(p) => p,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };

    let node_id = if let Some(status) = scx.extends.shared().await.session_status(&params.clientid).await {
        if status.online {
            status.id.node_id
        } else {
            res.render(StatusError::service_unavailable().detail("the session is offline"));
            return Ok(());
        }
    } else {
        res.render(StatusError::not_found().detail("session does not exist"));
        return Ok(());
    };

    if node_id == scx.node.id() {
        #[allow(clippy::mutable_key_type)]
        match subs::subscribe(scx, params).await {
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
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        // let cfg = get_cfg(depot)?;
        let message_type = cfg.read().await.message_type;
        //The session is on another node
        #[allow(clippy::mutable_key_type)]
        match _subscribe_on_other_node(scx, message_type, node_id, params).await {
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
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
async fn _subscribe_on_other_node(
    scx: &ServerContext,
    message_type: MessageType,
    node_id: NodeId,
    params: SubscribeParams,
) -> Result<HashMap<TopicFilter, (bool, Option<String>)>> {
    let c = get_grpc_client(scx, node_id).await?;
    let q = Message::Subscribe(params).encode()?;
    let reply = MessageSender::new(c, message_type, GrpcMessage::Data(q), Some(Duration::from_secs(15)))
        .send()
        .await?;
    match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res)? {
            MessageReply::Subscribe(ress) => Ok(ress),
            _ => unreachable!(),
        },
        reply => {
            log::info!("Subscribe GrpcMessage::Subscribe from other node({}), reply: {:?}", node_id, reply);
            Err(anyhow!("Invalid Operation"))
        }
    }
}

#[handler]
async fn unsubscribe(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let params = match req.parse_json::<UnsubscribeParams>().await {
        Ok(p) => p,
        Err(e) => {
            res.render(StatusError::bad_request().detail(e.to_string()));
            return Ok(());
        }
    };

    let node_id = if let Some(status) = scx.extends.shared().await.session_status(&params.clientid).await {
        if status.online {
            status.id.node_id
        } else {
            res.render(StatusError::service_unavailable().detail("the session is offline"));
            return Ok(());
        }
    } else {
        res.render(StatusError::not_found().detail("session does not exist"));
        return Ok(());
    };

    if node_id == scx.node.id() {
        match subs::unsubscribe(scx, params).await {
            Ok(()) => res.render(Json(true)),
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        // let cfg = get_cfg(depot)?;
        let message_type = cfg.read().await.message_type;
        //The session is on another node
        match _unsubscribe_on_other_node(scx, message_type, node_id, params).await {
            Ok(()) => res.render(Text::Plain("ok")),
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
async fn _unsubscribe_on_other_node(
    scx: &ServerContext,
    message_type: MessageType,
    node_id: NodeId,
    params: UnsubscribeParams,
) -> Result<()> {
    let c = get_grpc_client(scx, node_id).await?;
    let q = Message::Unsubscribe(params).encode()?;
    let reply = MessageSender::new(c, message_type, GrpcMessage::Data(q), Some(Duration::from_secs(15)))
        .send()
        .await?;
    match reply {
        GrpcMessageReply::Data(res) => match MessageReply::decode(&res)? {
            MessageReply::Unsubscribe => Ok(()),
            _ => unreachable!(),
        },
        reply => {
            log::info!(
                "Unsubscribe GrpcMessage::Unsubscribe from other node({}), reply: {:?}",
                node_id,
                reply
            );
            Err(anyhow!("Invalid Operation"))
        }
    }
}

#[handler]
async fn all_plugins(depot: &mut Depot, res: &mut Response) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;

    match _all_plugins(scx, message_type).await {
        Ok(pluginss) => res.render(Json(pluginss)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

#[inline]
async fn _all_plugins(scx: &ServerContext, message_type: MessageType) -> Result<Vec<serde_json::Value>> {
    let mut pluginss = Vec::new();
    let node_id = scx.node.id();
    let plugins = plugin::get_plugins(scx).await?;
    let plugins = plugins.into_iter().map(|p| p.to_json()).collect::<Result<Vec<_>>>()?;
    pluginss.push(json!({
        "node": node_id,
        "plugins": plugins,
    }));

    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::GetPlugins.encode()?;
        let replys = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
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
                Ok(_) => serde_json::Value::String("Invalid Result".into()),
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
async fn node_plugins(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    match _node_plugins(scx, node_id, message_type).await {
        Ok(plugins) => res.render(Json(plugins)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _node_plugins(
    scx: &ServerContext,
    node_id: NodeId,
    message_type: MessageType,
) -> Result<Vec<serde_json::Value>> {
    let plugins = if node_id == scx.node.id() {
        plugin::get_plugins(scx).await?
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::GetPlugins.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPlugins(plugins) => plugins,
                _ => unreachable!(),
            },
            reply => {
                log::info!("Get GrpcMessage::GetPlugins from other node({}), reply: {:?}", node_id, reply);
                return Err(anyhow!("Invalid Result"));
            }
        }
    };
    plugins.into_iter().map(|p| p.to_json()).collect::<Result<Vec<_>>>()
}

#[handler]
async fn node_plugin_info(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };

    match _node_plugin_info(scx, node_id, &name, message_type).await {
        Ok(plugin) => res.render(Json(plugin)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }

    Ok(())
}

async fn _node_plugin_info(
    scx: &ServerContext,
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<Option<serde_json::Value>> {
    let plugin = if node_id == scx.node.id() {
        plugin::get_plugin(scx, name).await?
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::GetPlugin { name }.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPlugin(plugin) => plugin,
                _ => unreachable!(),
            },
            reply => {
                log::info!("Get GrpcMessage::GetPlugin from other node({}), reply: {:?}", node_id, reply);
                return Err(anyhow!("Invalid Result"));
            }
        }
    };
    if let Some(plugin) = plugin {
        Ok(Some(plugin.to_json()?))
    } else {
        Ok(None)
    }
}

#[handler]
async fn node_plugin_config(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };

    match _node_plugin_config(scx, node_id, &name, message_type).await {
        Ok(cfg) => {
            res.headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json; charset=utf-8"));
            res.write_body(cfg).ok();
        }
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _node_plugin_config(
    scx: &ServerContext,
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<Vec<u8>> {
    let plugin_cfg = if node_id == scx.node.id() {
        plugin::get_plugin_config(scx, name).await?
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::GetPluginConfig { name }.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::GetPluginConfig(cfg) => cfg,
                _ => unreachable!(),
            },
            reply => {
                log::info!(
                    "Get GrpcMessage::GetPluginConfig from other node({}), reply: {:?}",
                    node_id,
                    reply
                );
                return Err(anyhow!("Invalid Result"));
            }
        }
    };
    Ok(plugin_cfg)
}

#[handler]
async fn node_plugin_config_reload(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };

    match _node_plugin_config_reload(scx, node_id, &name, message_type).await {
        Ok(r) => res.render(Json(r)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _node_plugin_config_reload(
    scx: &ServerContext,
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<bool> {
    if node_id == scx.node.id() {
        scx.plugins.load_config(name).await?;
        Ok(true)
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::ReloadPluginConfig { name }.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(15)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::ReloadPluginConfig => Ok(true),
                _ => unreachable!(),
            },
            reply => {
                log::info!(
                    "ConfigReload GrpcMessage::ReloadPluginConfig from other node({}), reply: {:?}",
                    node_id,
                    reply
                );
                Ok(false)
            }
        }
    }
}

#[handler]
async fn node_plugin_load(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };

    match _node_plugin_load(scx, node_id, &name, message_type).await {
        Ok(r) => res.render(Json(r)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _node_plugin_load(
    scx: &ServerContext,
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<bool> {
    if node_id == scx.node.id() {
        scx.plugins.start(name).await?;
        Ok(true)
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::LoadPlugin { name }.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::LoadPlugin => Ok(true),
                _ => unreachable!(),
            },
            reply => {
                log::info!("Load GrpcMessage::LoadPlugin from other node({}), reply: {:?}", node_id, reply);
                Ok(false)
            }
        }
    }
}

#[handler]
async fn node_plugin_unload(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    //let cfg = get_cfg(depot)?;
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;
    let node_id = if let Some(node_id) = req.param::<NodeId>("node") {
        node_id
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };
    let name = if let Some(name) = req.param::<String>("plugin") {
        name
    } else {
        res.status_code(StatusCode::NOT_FOUND);
        return Ok(());
    };

    match _node_plugin_unload(scx, node_id, &name, message_type).await {
        Ok(r) => res.render(Json(r)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _node_plugin_unload(
    scx: &ServerContext,
    node_id: NodeId,
    name: &str,
    message_type: MessageType,
) -> Result<bool> {
    if node_id == scx.node.id() {
        scx.plugins.stop(name).await
    } else {
        let c = get_grpc_client(scx, node_id).await?;
        let msg = Message::UnloadPlugin { name }.encode()?;
        let reply =
            MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                .send()
                .await?;
        match reply {
            GrpcMessageReply::Data(msg) => match MessageReply::decode(&msg)? {
                MessageReply::UnloadPlugin(ok) => Ok(ok),
                _ => unreachable!(),
            },
            reply => {
                log::info!(
                    "Unload GrpcMessage::UnloadPlugin from other node({}), reply: {:?}",
                    node_id,
                    reply
                );
                Ok(false)
            }
        }
    }
}

#[handler]
async fn get_stats_sum(depot: &mut Depot, res: &mut Response) -> std::result::Result<(), salvo::Error> {
    // let cfg = get_cfg(depot)?;
    let (scx, cfg) = get_scx_cfg(depot)?;

    let message_type = cfg.read().await.message_type;

    match _get_stats_sum(scx, message_type).await {
        Ok(stats_sum) => res.render(Json(stats_sum)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _get_stats_sum(scx: &ServerContext, message_type: MessageType) -> Result<serde_json::Value> {
    let this_id = scx.node.id();
    let mut nodes = HashMap::default();
    nodes.insert(
        this_id,
        json!({
            "name": scx.node.name(scx,this_id).await,
            "running": scx.node.status(scx).await.is_running(),
        }),
    );

    let mut stats_sum = scx.stats.clone(scx).await;
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        {
            match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => {
                        nodes.insert(
                            id,
                            json!({
                                "name": scx.node.name(scx, id).await,
                                "running": node_status.is_running(),
                            }),
                        );
                        stats_sum.add(*stats);
                    }
                    _ => unreachable!(),
                },
                (id, Ok(reply)) => {
                    log::info!("Get GrpcMessage::StateInfo from other node({}), reply: {:?}", id, reply);
                    continue;
                }
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node({}), error: {:?}", id, e);
                    nodes.insert(id, serde_json::Value::String(e.to_string()));
                }
            };
        }
    }

    let stats_sum = json!({
        "nodes": nodes,
        "stats": stats_sum.to_json(scx).await
    });

    Ok(stats_sum)
}

#[handler]
async fn get_stats(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    //let cfg = get_cfg(depot)?;
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match get_stats_one(scx, message_type, id).await {
            Ok(Some((node_status, stats))) => {
                let stat_info = _build_stats(scx, id, node_status, stats.to_json(scx).await).await;
                res.render(Json(stat_info))
            }
            Ok(None) => {
                //| Err(MqttError::None)
                res.status_code(StatusCode::NOT_FOUND);
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        match get_stats_all(scx, message_type).await {
            Ok(stats) => {
                let mut stat_infos = Vec::new();
                for item in stats {
                    match item {
                        Ok((id, node_status, state)) => {
                            stat_infos
                                .push(_build_stats(scx, id, node_status, state.to_json(scx).await).await);
                        }
                        Err(e) => {
                            stat_infos.push(serde_json::Value::String(e.to_string()));
                        }
                    }
                }
                res.render(Json(stat_infos))
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
pub(crate) async fn get_stats_one(
    scx: &ServerContext,
    message_type: MessageType,
    id: NodeId,
) -> Result<Option<(NodeStatus, Box<Stats>)>> {
    if id == scx.node.id() {
        let node_status = scx.node.status(scx).await;
        let stats = scx.stats.clone(scx).await;
        Ok(Some((node_status, Box::new(stats))))
    } else {
        let grpc_clients = scx.extends.shared().await.get_grpc_clients();
        if let Some(c) = grpc_clients.get(&id).map(|(_, c)| c.clone()) {
            let msg = Message::StatsInfo.encode()?;
            let reply =
                MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                    .send()
                    .await;
            match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => Ok(Some((node_status, stats))),
                    _ => unreachable!(),
                },
                Ok(reply) => {
                    log::info!("Get GrpcMessage::StateInfo from other node, reply: {:?}", reply);
                    Err(anyhow!("Invalid Result"))
                }
                Err(e) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node, error: {:?}", e);
                    Err(e)
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[inline]
pub(crate) async fn get_stats_all(
    scx: &ServerContext,
    message_type: MessageType,
) -> Result<Vec<Result<(NodeId, NodeStatus, Box<Stats>)>>> {
    let id = scx.node.id();
    let node_status = scx.node.status(scx).await;
    let state = scx.stats.clone(scx).await;
    //let mut stats = vec![_build_stats(id, node_status, state).await];
    let mut stats = vec![Ok((id, node_status, Box::new(state)))];

    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::StatsInfo(node_status, stats) => Ok((id, node_status, stats)),
                    _ => unreachable!(),
                },
                (id, Ok(reply)) => {
                    log::info!("Get GrpcMessage::StateInfo from other node({}), reply: {:?}", id, reply);
                    continue;
                }
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::StateInfo from other node({}), error: {:?}", id, e);
                    Err(e)
                }
            };
            stats.push(data);
        }
    }
    Ok(stats)
}

#[inline]
async fn _build_stats(
    scx: &ServerContext,
    id: NodeId,
    node_status: NodeStatus,
    stats: serde_json::Value,
) -> serde_json::Value {
    let node_name = scx.node.name(scx, id).await;
    let data = json!({
        "node": {
            "id": id,
            "name": node_name,
            "running": node_status.is_running(),
        },
        "stats": stats
    });
    data
}

#[handler]
async fn get_metrics(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;

    let message_type = cfg.read().await.message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match get_metrics_one(scx, message_type, id).await {
            Ok(Some(metrics)) => {
                let metrics = _build_metrics(scx, id, metrics.to_json()).await;
                res.render(Json(metrics))
            }
            Ok(None) => {
                res.status_code(StatusCode::NOT_FOUND);
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        match get_metrics_all(scx, message_type).await {
            Ok(items) => {
                let mut metrics_infos = Vec::new();
                for item in items {
                    match item {
                        Ok((id, metrics)) => {
                            metrics_infos.push(_build_metrics(scx, id, metrics.to_json()).await);
                        }
                        Err(e) => {
                            metrics_infos.push(serde_json::Value::String(e.to_string()));
                        }
                    }
                }
                res.render(Json(metrics_infos))
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[inline]
pub(crate) async fn get_metrics_one(
    scx: &ServerContext,
    message_type: MessageType,
    id: NodeId,
) -> Result<Option<Box<Metrics>>> {
    if id == scx.node.id() {
        // let metrics = scx.metrics;
        Ok(Some(Box::new(scx.metrics.clone())))
    } else {
        let grpc_clients = scx.extends.shared().await.get_grpc_clients();
        if let Some(c) = grpc_clients.get(&id).map(|(_, c)| c.clone()) {
            let msg = Message::MetricsInfo.encode()?;
            let reply =
                MessageSender::new(c, message_type, GrpcMessage::Data(msg), Some(Duration::from_secs(10)))
                    .send()
                    .await;
            match reply {
                Ok(GrpcMessageReply::Data(msg)) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => Ok(Some(metrics)),
                    _ => unreachable!(),
                },
                Ok(reply) => {
                    log::info!("Get GrpcMessage::MetricsInfo from other node, reply: {:?}", reply);
                    Err(anyhow!("Invalid Result"))
                }
                Err(e) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node, error: {:?}", e);
                    Err(e)
                }
            }
        } else {
            Ok(None)
        }
    }
}

#[inline]
pub(crate) async fn get_metrics_all(
    scx: &ServerContext,
    message_type: MessageType,
) -> Result<Vec<Result<(NodeId, Box<Metrics>)>>> {
    let id = scx.node.id();
    let mut metricses = vec![Ok((id, Box::new(scx.metrics.clone())))];

    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::MetricsInfo.encode()?;
        let replys = MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await;
        for reply in replys {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => Ok((id, metrics)),
                    _ => unreachable!(),
                },
                (id, Ok(reply)) => {
                    log::info!("Get GrpcMessage::MetricsInfo from other node({}), reply: {:?}", id, reply);
                    continue;
                }
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node({}), error: {:?}", id, e);
                    Err(e)
                }
            };
            metricses.push(data);
        }
    }
    Ok(metricses)
}

#[handler]
async fn get_metrics_sum(depot: &mut Depot, res: &mut Response) -> std::result::Result<(), salvo::Error> {
    let (scx, cfg) = get_scx_cfg(depot)?;
    let message_type = cfg.read().await.message_type;

    match _get_metrics_sum(scx, message_type).await {
        Ok(metrics_sum) => res.render(Json(metrics_sum)),
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

async fn _get_metrics_sum(scx: &ServerContext, message_type: MessageType) -> Result<serde_json::Value> {
    let mut metrics_sum = scx.metrics.clone();
    let grpc_clients = scx.extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::MetricsInfo.encode()?;
        for reply in MessageBroadcaster::new(
            grpc_clients,
            message_type,
            GrpcMessage::Data(msg),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await
        {
            match reply {
                (_, Ok(GrpcMessageReply::Data(msg))) => match MessageReply::decode(&msg)? {
                    MessageReply::MetricsInfo(metrics) => metrics_sum.add(&metrics),
                    _ => unreachable!(),
                },
                (id, Ok(reply)) => {
                    log::info!("Get GrpcMessage::MetricsInfo from other node({}), reply: {:?}", id, reply);
                }
                (id, Err(e)) => {
                    log::warn!("Get GrpcMessage::MetricsInfo from other node({}), error: {:?}", id, e);
                }
            };
        }
    }

    Ok(metrics_sum.to_json())
}

#[inline]
async fn _build_metrics(scx: &ServerContext, id: NodeId, metrics: serde_json::Value) -> serde_json::Value {
    let node_name = scx.node.name(scx, id).await;
    let data = json!({
        "node": {
            "id": id,
            "name": node_name,
        },
        "metrics": metrics
    });
    data
}

#[handler]
async fn get_prometheus_metrics(
    req: &mut Request,
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    // let cfg = get_cfg(depot)?;
    let (scx, cfg) = get_scx_cfg(depot)?;

    let (message_type, cache_interval) = {
        let cfg_rl = cfg.read().await;
        (cfg_rl.message_type, cfg_rl.prometheus_metrics_cache_interval)
    };
    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match prome::to_metrics(scx, message_type, cache_interval, PrometheusDataType::Node(id)).await {
            Ok(metrics) => {
                res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
                res.write_body(metrics).ok();
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    } else {
        match prome::to_metrics(scx, message_type, cache_interval, PrometheusDataType::All).await {
            Ok(metrics) => {
                res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
                res.write_body(metrics).ok();
            }
            Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
        }
    }
    Ok(())
}

#[handler]
async fn get_prometheus_metrics_sum(
    depot: &mut Depot,
    res: &mut Response,
) -> std::result::Result<(), salvo::Error> {
    // let scx = get_scx(depot)?;
    // let cfg = get_cfg(depot)?;
    let (scx, cfg) = get_scx_cfg(depot)?;
    let (message_type, cache_interval) = {
        let cfg_rl = cfg.read().await;
        (cfg_rl.message_type, cfg_rl.prometheus_metrics_cache_interval)
    };
    match prome::to_metrics(scx, message_type, cache_interval, PrometheusDataType::Sum).await {
        Ok(metrics) => {
            res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            res.write_body(metrics).ok();
        }
        Err(e) => res.render(StatusError::service_unavailable().detail(e.to_string())),
    }
    Ok(())
}

#[inline]
async fn get_grpc_client(scx: &ServerContext, node_id: NodeId) -> Result<GrpcClient> {
    scx.extends
        .shared()
        .await
        .get_grpc_clients()
        .get(&node_id)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| anyhow!("node grpc client is not exist!"))
}
