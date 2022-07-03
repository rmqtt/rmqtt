use std::net::SocketAddr;

use salvo::extra::affix;
use salvo::prelude::*;

use rmqtt::{anyhow, HashMap, log, serde_json::{self, json}, tokio::sync::oneshot};
use rmqtt::{
    broker::types::NodeId,
    grpc::{Message, MessageBroadcaster, MessageReply, MessageSender},
    node::NodeStatus,
    Result,
    Runtime,
};
use rmqtt::grpc::MessageType;

use super::PluginConfigType;

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
        if let Some(broker_info) = _get_broker(message_type, id).await {
            res.render(Json(broker_info));
        } else {
            res.set_status_code(StatusCode::NOT_FOUND)
        }
    } else {
        let broker_infos = _get_brokers(message_type).await;
        res.render(Json(broker_infos));
    }
}

#[inline]
async fn _get_broker(message_type: MessageType, id: NodeId) -> Option<serde_json::Value> {
    if id == Runtime::instance().node.id() {
        Some(Runtime::instance().node.broker_info().await.to_json())
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let reply = MessageSender::new(c.clone(), message_type, Message::BrokerInfo)
                .send().await;
            let broker_info = match reply {
                Ok(MessageReply::BrokerInfo(broker_info)) => broker_info.to_json(),
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get Message::BrokerInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Some(broker_info)
        } else {
            None
        }
    }
}

#[inline]
async fn _get_brokers(message_type: MessageType) -> Vec<serde_json::Value> {
    let mut brokers = vec![Runtime::instance().node.broker_info().await.to_json()];
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, Message::BrokerInfo)
                .join_all()
                .await
                .drain(..)
                .map(|reply| match reply {
                    (_, Ok(MessageReply::BrokerInfo(broker_info))) => broker_info.to_json(),
                    (_, Ok(_)) => unreachable!(),
                    (id, Err(e)) => {
                        log::warn!("Get Message::BrokerInfo from other node({}), error: {:?}", id, e);
                        serde_json::Value::String(e.to_string())
                    }
                })
                .collect::<Vec<_>>();
        brokers.extend(replys);
    }
    brokers
}

#[fn_handler]
async fn get_nodes(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        if let Some(node_info) = _get_node(message_type, id).await {
            res.render(Json(node_info));
        } else {
            res.set_status_code(StatusCode::NOT_FOUND)
        }
    } else {
        let node_infos = _get_nodes(message_type).await;
        res.render(Json(node_infos));
    }
}

#[inline]
async fn _get_node(message_type: MessageType, id: NodeId) -> Option<serde_json::Value> {
    if id == Runtime::instance().node.id() {
        Some(Runtime::instance().node.node_info().await.to_json())
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let reply = MessageSender::new(c.clone(), message_type, Message::NodeInfo)
                .send().await;
            let node_info = match reply {
                Ok(MessageReply::NodeInfo(node_info)) => node_info.to_json(),
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get Message::NodeInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Some(node_info)
        } else {
            None
        }
    }
}

#[inline]
async fn _get_nodes(message_type: MessageType) -> Vec<serde_json::Value> {
    let mut nodes = vec![Runtime::instance().node.node_info().await.to_json()];
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, Message::NodeInfo)
                .join_all()
                .await
                .drain(..)
                .map(|reply| match reply {
                    (_, Ok(MessageReply::NodeInfo(node_info))) => node_info.to_json(),
                    (_, Ok(_)) => unreachable!(),
                    (id, Err(e)) => {
                        log::warn!("Get Message::NodeInfo from other node({}), error: {:?}", id, e);
                        serde_json::Value::String(e.to_string())
                    }
                })
                .collect::<Vec<_>>();
        nodes.extend(replys);
    }
    nodes
}

#[fn_handler]
async fn get_stats_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let this_id = Runtime::instance().node.id();
    let mut nodes = HashMap::default();
    nodes.insert(this_id, json!({
        "name": Runtime::instance().node.name(this_id).await,
        "status": Runtime::instance().node.status().await,
    }));

    let mut stats_sum = Runtime::instance().stats.clone();
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        for reply in MessageBroadcaster::new(grpc_clients, message_type, Message::StateInfo)
            .join_all()
            .await {
            match reply {
                (id, Ok(MessageReply::StateInfo(node_status, stats))) => {
                    nodes.insert(id, json!({
                        "name": Runtime::instance().node.name(id).await,
                        "status": node_status,
                    }));
                    stats_sum.add(*stats);
                }
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get Message::StateInfo from other node({}), error: {:?}", id, e);
                    nodes.insert(id, serde_json::Value::String(e.to_string()));
                }
            };
        }
    }

    let stats_sum = json!({
        "nodes": nodes,
        "stats": stats_sum.to_json()
    });

    res.render(Json(stats_sum));
}

#[fn_handler]
async fn get_stats(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let id = req.param::<NodeId>("id");
    if let Some(id) = id {
        if let Some(stat_info) = _get_stats_one(message_type, id).await {
            res.render(Json(stat_info));
        } else {
            res.set_status_code(StatusCode::NOT_FOUND)
        }
    } else {
        let stat_infos = _get_stats_all(message_type).await;
        res.render(Json(stat_infos));
    }
}

#[inline]
async fn _get_stats_one(message_type: MessageType, id: NodeId) -> Option<serde_json::Value> {
    if id == Runtime::instance().node.id() {
        let node_status = Runtime::instance().node.status().await;
        let stats = Runtime::instance().stats;
        Some(_build_stats(id, node_status, stats.to_json()).await)
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let reply = MessageSender::new(c.clone(), message_type, Message::StateInfo)
                .send().await;
            let stats = match reply {
                Ok(MessageReply::StateInfo(node_status, stats)) => _build_stats(id, node_status, stats.to_json()).await,
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get Message::StateInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Some(stats)
        } else {
            None
        }
    }
}

#[inline]
async fn _get_stats_all(message_type: MessageType) -> Vec<serde_json::Value> {
    let id = Runtime::instance().node.id();
    let node_status = Runtime::instance().node.status().await;
    let state = Runtime::instance().stats;
    let mut stats = vec![_build_stats(id, node_status, state.to_json()).await];

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        for reply in MessageBroadcaster::new(grpc_clients, message_type, Message::StateInfo)
            .join_all()
            .await {
            let data = match reply {
                (id, Ok(MessageReply::StateInfo(node_status, state))) => {
                    _build_stats(id, node_status, state.to_json()).await
                }
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get Message::StateInfo from other node({}), error: {:?}", id, e);
                    serde_json::Value::String(e.to_string())
                }
            };
            stats.push(data);
        }
    }
    stats
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
        if let Some(metrics) = _get_metrics_one(message_type, id).await {
            res.render(Json(metrics));
        } else {
            res.set_status_code(StatusCode::NOT_FOUND)
        }
    } else {
        let metricses = _get_metrics_all(message_type).await;
        res.render(Json(metricses));
    }
}


#[inline]
async fn _get_metrics_one(message_type: MessageType, id: NodeId) -> Option<serde_json::Value> {
    if id == Runtime::instance().node.id() {
        let metrics = Runtime::instance().metrics.to_json();
        Some(_build_metrics(id, metrics).await)
    } else {
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if let Some((_, c)) = grpc_clients.get(&id) {
            let reply = MessageSender::new(c.clone(), message_type, Message::MetricsInfo)
                .send().await;
            let metrics = match reply {
                Ok(MessageReply::MetricsInfo(metrics)) => _build_metrics(id, metrics.to_json()).await,
                Ok(_) => unreachable!(),
                Err(e) => {
                    log::warn!("Get Message::MetricsInfo from other node, error: {:?}", e);
                    serde_json::Value::String(e.to_string())
                }
            };
            Some(metrics)
        } else {
            None
        }
    }
}

#[inline]
async fn _get_metrics_all(message_type: MessageType) -> Vec<serde_json::Value> {
    let id = Runtime::instance().node.id();
    let metrics = Runtime::instance().metrics.to_json();
    let mut metricses = vec![_build_metrics(id, metrics).await];

    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        let replys =
            MessageBroadcaster::new(grpc_clients, message_type, Message::MetricsInfo)
                .join_all()
                .await;
        for reply in replys {
            let data = match reply {
                (id, Ok(MessageReply::MetricsInfo(metrics))) => _build_metrics(id, metrics.to_json()).await,
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get Message::MetricsInfo from other node({}), error: {:?}", id, e);
                    serde_json::Value::String(e.to_string())
                }
            };
            metricses.push(data);
        }
    }
    metricses
}

#[fn_handler]
async fn get_metrics_sum(depot: &mut Depot, res: &mut Response) {
    let cfg = depot.obtain::<PluginConfigType>().cloned().unwrap();
    let message_type = cfg.read().message_type;

    let mut metrics_sum = Runtime::instance().metrics.clone();
    let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
    if !grpc_clients.is_empty() {
        for reply in MessageBroadcaster::new(grpc_clients, message_type, Message::MetricsInfo)
            .join_all()
            .await {
            match reply {
                (_id, Ok(MessageReply::MetricsInfo(metrics))) => {
                    metrics_sum.add(&metrics);
                }
                (_, Ok(_)) => unreachable!(),
                (id, Err(e)) => {
                    log::warn!("Get Message::MetricsInfo from other node({}), error: {:?}", id, e);
                    //nodes.insert(id, serde_json::Value::String(e.to_string()));
                }
            };
        }
    }

    res.render(Json(metrics_sum.to_json()));
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