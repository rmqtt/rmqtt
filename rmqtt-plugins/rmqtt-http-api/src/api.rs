use std::net::SocketAddr;

use salvo::extra::affix;
use salvo::prelude::*;

use rmqtt::{anyhow, HashMap, log, MqttError, serde_json::{self, json}, tokio::sync::oneshot};
use rmqtt::{
    broker::types::NodeId,
    grpc::{Message as GrpcMessage,
           MessageBroadcaster,
           MessageReply as GrpcMessageReply,
           MessageSender,
           MessageType},
    node::NodeStatus,
    Result,
    Runtime,
};

use super::clients;
use super::PluginConfigType;
use super::types::{ClientSearchParams, Message, MessageReply};

fn route(cfg: PluginConfigType) -> Router {
    Router::with_path("api/v1")
        .hoop(affix::inject(cfg))
        .get(list_apis)
        .push(
            Router::with_path("brokers")
                .get(get_brokers)
                .push(Router::with_path("<id>").get(get_brokers))
        )
        .push(
            Router::with_path("nodes")
                .get(get_nodes)
                .push(Router::with_path("<id>").get(get_nodes))
        )
        .push(
            Router::with_path("clients")
                .get(search_clients)
                .push(Router::with_path("<clientid>").get(get_client))
        )
        .push(
            Router::with_path("stats")
                .get(get_stats)
                .push(Router::with_path("sum").get(get_stats_sum))
                .push(Router::with_path("<id>").get(get_stats))
        )
        .push(
            Router::with_path("metrics")
                .get(get_metrics)
                .push(Router::with_path("sum").get(get_metrics_sum))
                .push(Router::with_path("<id>").get(get_metrics))
        )
}

pub(crate) async fn listen_and_serve(laddr: SocketAddr, cfg: PluginConfigType, rx: oneshot::Receiver<()>) -> Result<()> {
    log::info!("HTTP API Listening on {}", laddr);
    Server::new(TcpListener::bind(laddr))
        .try_serve_with_graceful_shutdown(route(cfg), async {
            rx.await.ok();
        }).await.map_err(anyhow::Error::new)?;
    Ok(())
}


#[fn_handler]
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

#[fn_handler]
async fn get_brokers(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_broker(message_type, id).await {
            Ok(Some(broker_info)) => res.render(Json(broker_info)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    } else {
        match _get_brokers(message_type).await {
            Ok(brokers) => res.render(Json(brokers)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
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
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(msg))
                .send().await;
            let broker_info = match reply {
                Ok(GrpcMessageReply::Data(msg)) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::BrokerInfo(broker_info) => broker_info.to_json(),
                        _ => unreachable!()
                    }
                }
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
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
                .join_all()
                .await
                .drain(..)
                .map(|reply| match reply {
                    (_, Ok(GrpcMessageReply::Data(msg))) => {
                        match MessageReply::decode(&msg) {
                            Ok(MessageReply::BrokerInfo(broker_info)) => Ok(broker_info.to_json()),
                            Err(e) => Err(e),
                            _ => unreachable!()
                        }
                    }
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

#[fn_handler]
async fn get_nodes(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_node(message_type, id).await {
            Ok(Some(node_info)) => res.render(Json(node_info)),
            Ok(None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    } else {
        match _get_nodes(message_type).await {
            Ok(node_infos) => res.render(Json(node_infos)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
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
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(msg))
                .send().await;
            let node_info = match reply {
                Ok(GrpcMessageReply::Data(msg)) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::NodeInfo(node_info) => node_info.to_json(),
                        _ => unreachable!()
                    }
                }
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
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
                .join_all()
                .await
                .drain(..)
                .map(|reply| match reply {
                    (_, Ok(GrpcMessageReply::Data(msg))) => {
                        match MessageReply::decode(&msg) {
                            Ok(MessageReply::NodeInfo(node_info)) => Ok(node_info.to_json()),
                            Err(e) => Err(e),
                            _ => unreachable!()
                        }
                    }
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

#[fn_handler]
async fn get_client(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;
    let clientid = req.param::<String>("clientid");
    if let Some(clientid) = clientid {
        match _get_client(message_type, &clientid).await {
            Ok(Some(reply)) => {
                res.render(Json(reply))
            }
            Ok(None) | Err(MqttError::None) => {
                res.set_status_code(StatusCode::NOT_FOUND)
            }
            Err(e) => {
                res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
            }
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

    let check_result = |reply: GrpcMessageReply| {
        match reply {
            GrpcMessageReply::Data(res) => {
                match MessageReply::decode(&res) {
                    Ok(MessageReply::ClientGet(ress)) => {
                        match ress {
                            Some(res) => {
                                Ok(res)
                            }
                            None => {
                                Err(MqttError::None)
                            }
                        }
                    }
                    Err(e) => {
                        Err(e)
                    }
                    _ => unreachable!()
                }
            }
            _ => unreachable!(),
        }
    };

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let q = Message::ClientGet { clientid }.encode()?;
        let reply =
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(q))
                .select_ok(check_result)
                .await?;
        return Ok(Some(reply.to_json()));
    }

    Ok(None)
}

#[fn_handler]
async fn search_clients(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;
    let max_row_limit = cfg.read().max_row_limit;
    let mut q = match req.parse_queries::<ClientSearchParams>() {
        Ok(q) => q,
        Err(e) => return res.set_status_error(StatusError::bad_request().with_detail(e.to_string()))
    };

    if q._limit == 0 || q._limit > max_row_limit {
        q._limit = max_row_limit;
    }
    match _search_clients(message_type, q).await {
        Ok(replys) => {
            res.render(Json(replys))
        }
        Err(e) => {
            res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    }
}

async fn _search_clients(message_type: MessageType, mut q: ClientSearchParams) -> Result<Vec<serde_json::Value>> {
    let mut replys = clients::search(&q).await;
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    for (_id, (_addr, c)) in grpc_clients.iter() {
        if replys.len() < q._limit {
            q._limit -= replys.len();

            let q = Message::ClientSearch(q.clone()).encode()?;
            let reply = MessageSender::new(c.clone(), message_type, GrpcMessage::Data(q))
                .send().await;
            match reply {
                Ok(GrpcMessageReply::Data(res)) => {
                    match MessageReply::decode(&res)? {
                        MessageReply::ClientSearch(ress) => {
                            replys.extend(ress);
                        }
                        _ => unreachable!()
                    }
                }
                Err(e) => {
                    log::warn!("_search_clients, error: {:?}", e);
                }
                _ => unreachable!(),
            };
        } else {
            break;
        }
    }

    let replys = replys.iter()
        .map(|res| res.to_json()).collect::<Vec<_>>();
    Ok(replys)
}

#[fn_handler]
async fn get_stats_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    match _get_stats_sum(message_type).await {
        Ok(stats_sum) => res.render(Json(stats_sum)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
    }
}

async fn _get_stats_sum(message_type: MessageType) -> Result<serde_json::Value> {
    let this_id = Runtime::instance().node.id();
    let mut nodes = HashMap::default();
    nodes.insert(this_id, json!({
        "name": Runtime::instance().node.name(this_id).await,
        "status": Runtime::instance().node.status().await,
    }));

    let mut stats_sum = Runtime::instance().stats.clone().await;
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await {
            match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::StatsInfo(node_status, stats) => {
                            nodes.insert(id, json!({
                                "name": Runtime::instance().node.name(id).await,
                                "status": node_status,
                            }));
                            stats_sum.add(*stats);
                        }
                        _ => unreachable!()
                    }
                }
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
        "stats": stats_sum.to_json()
    });

    Ok(stats_sum)
}

#[fn_handler]
async fn get_stats(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_stats_one(message_type, id).await {
            Ok(Some(stat_info)) => res.render(Json(stat_info)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    } else {
        match _get_stats_all(message_type).await {
            Ok(stat_infos) => res.render(Json(stat_infos)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    }
}

#[inline]
async fn _get_stats_one(message_type: MessageType, id: NodeId) -> Result<Option<serde_json::Value>> {
    if id == Runtime::instance().node.id() {
        let node_status = Runtime::instance().node.status().await;
        let stats = Runtime::instance().stats.clone().await;
        Ok(Some(_build_stats(id, node_status, stats.to_json()).await))
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some(c) = grpc_clients.get(&id).map(|(_, c)| c.clone()) {
            let msg = Message::StatsInfo.encode()?;
            let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg))
                .send().await;
            let stats = match reply {
                Ok(GrpcMessageReply::Data(msg)) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::StatsInfo(node_status, stats) => {
                            _build_stats(id, node_status, stats.to_json()).await
                        }
                        _ => unreachable!()
                    }
                }
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
    let mut stats = vec![_build_stats(id, node_status, state.to_json()).await];

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::StatsInfo.encode()?;
        for reply in MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::StatsInfo(node_status, stats) => _build_stats(id, node_status, stats.to_json()).await,
                        _ => unreachable!()
                    }
                }
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

#[fn_handler]
async fn get_metrics(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        match _get_metrics_one(message_type, id).await {
            Ok(Some(metrics)) => res.render(Json(metrics)),
            Ok(None) | Err(MqttError::None) => res.set_status_code(StatusCode::NOT_FOUND),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
        }
    } else {
        match _get_metrics_all(message_type).await {
            Ok(metricses) => res.render(Json(metricses)),
            Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
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
            let reply = MessageSender::new(c, message_type, GrpcMessage::Data(msg))
                .send().await;
            let metrics = match reply {
                Ok(GrpcMessageReply::Data(msg)) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::MetricsInfo(metrics) => _build_metrics(id, metrics.to_json()).await,
                        _ => unreachable!()
                    }
                }
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
            MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
                .join_all()
                .await;
        for reply in replys {
            let data = match reply {
                (id, Ok(GrpcMessageReply::Data(msg))) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::MetricsInfo(metrics) => _build_metrics(id, metrics.to_json()).await,
                        _ => unreachable!()
                    }
                }
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

#[fn_handler]
async fn get_metrics_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    match _get_metrics_sum(message_type).await {
        Ok(metrics_sum) => res.render(Json(metrics_sum)),
        Err(e) => res.set_status_error(StatusError::service_unavailable().with_detail(e.to_string()))
    }
}

async fn _get_metrics_sum(message_type: MessageType) -> Result<serde_json::Value> {
    let mut metrics_sum = Runtime::instance().metrics.clone();
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let msg = Message::MetricsInfo.encode()?;
        for reply in MessageBroadcaster::new(grpc_clients, message_type, GrpcMessage::Data(msg))
            .join_all()
            .await {
            match reply {
                (_id, Ok(GrpcMessageReply::Data(msg))) => {
                    match MessageReply::decode(&msg)? {
                        MessageReply::MetricsInfo(metrics) => metrics_sum.add(&metrics),
                        _ => unreachable!()
                    }
                }
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


