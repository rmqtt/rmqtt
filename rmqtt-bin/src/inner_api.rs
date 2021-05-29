use std::net::SocketAddr;
use warp::http::StatusCode;
use warp::Filter;

use rmqtt::broker::types::{ClientId, Id};
use rmqtt::grpc::server::active_grpc_requests;
use rmqtt::{serde::ser::Serialize, serde_json};
use rmqtt::{Result, Runtime};

#[allow(dead_code)]
mod version {
    include!(concat!(env!("OUT_DIR"), "/version.rs"));
}

pub async fn serve<T: Into<SocketAddr>>(laddr: T) -> Result<()> {
    let root = || warp::any();

    //version, /version
    let version = root()
        .and(warp::path("version"))
        .and(warp::path::end())
        .map(|| warp::reply::with_status(version::VERSION, StatusCode::OK));

    //mqtt server current status, /status
    let status = root()
        .and(warp::path("status"))
        .and(warp::path::end())
        .and_then(|| async move { with_reply_json(status().await) });

    //session info, /session/:client_id
    let session = root()
        .and(warp::path!("session" / String))
        .and(warp::path::end())
        .and_then(|client_id: String| async move { with_reply_json(session(client_id).await) });

    //random get one session info, /random_session
    let random_session =
        root().and(warp::path("random_session")).and(warp::path::end()).and_then(|| async move {
            match random_session().await {
                Ok(data) => with_reply_json(data),
                Err(e) => {
                    with_reply_status(warp::http::StatusCode::INTERNAL_SERVER_ERROR, Some(format!("{:?}", e)))
                }
            }
        });

    //router list, /router/list/:top
    let router_list = root()
        .and(warp::path!("router" / "list" / usize))
        .and(warp::path::end())
        .and_then(|top: usize| async move { with_reply_string(list_routers(top).await) })
        .or(root()
            .and(warp::path!("router" / "list"))
            .and(warp::path::end())
            .and_then(|| async move { with_reply_string(list_routers(1000).await) }));

    //plugin list, /plugin/list
    let plugin_list = root()
        .and(warp::path!("plugin" / "list"))
        .and(warp::path::end())
        .and_then(|| async move { with_reply_json(plugin_list().await) });

    //plugin config, /plugin/config
    let plugin_config = root()
        .and(warp::path!("plugin" / "config" / String))
        .and(warp::path::end())
        .and_then(|name: String| async move {
            match plugin_config(&name).await {
                Ok(cfg) => with_reply_json(cfg),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        });

    let get_apis = warp::get().and(
        version.or(status).or(session).or(random_session).or(router_list).or(plugin_list).or(plugin_config),
    );

    let routes = get_apis;
    warp::serve(routes).try_bind(laddr).await;
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn with_reply_string<T: Into<String>>(data: T) -> std::result::Result<Box<dyn warp::Reply>, warp::Rejection> {
    Ok(Box::new(warp::reply::with_status(data.into(), StatusCode::OK)))
}

#[allow(clippy::unnecessary_wraps)]
fn with_reply_json<T: Serialize>(data: T) -> std::result::Result<Box<dyn warp::Reply>, warp::Rejection> {
    Ok(Box::new(warp::reply::json(&data)))
}

#[allow(clippy::unnecessary_wraps)]
fn with_reply_status(
    status: warp::http::StatusCode,
    reason: Option<String>,
) -> std::result::Result<Box<dyn warp::Reply>, warp::Rejection> {
    Ok(Box::new(warp::reply::with_status(reason.unwrap_or_else(|| status.to_string()), status)))
}

async fn status() -> serde_json::Value {
    let shared = Runtime::instance().extends.shared().await;
    serde_json::json!({
        "sessions": shared.sessions().await,
        "clients": shared.clients().await,
        "active_grpc_requestss": active_grpc_requests()
    })
}

async fn session(id: String) -> serde_json::Value {
    let entry = Runtime::instance()
        .extends
        .shared()
        .await
        .entry(Id::from(Runtime::instance().node.id(), ClientId::from(id)));

    let session_info = if let Some(s) = entry.session().await { Some(s.to_json().await) } else { None };

    let client_info = if let Some(c) = entry.client().await { Some(c.to_json().await) } else { None };

    serde_json::json!({
        "session": session_info,
        "client": client_info,
    })
}

async fn random_session() -> Result<Option<serde_json::Value>> {
    let data = match Runtime::instance().extends.shared().await.random_session() {
        Some((s, c)) => Some(serde_json::json!({
            "session": s.to_json().await,
            "client": c.to_json().await
        })),
        None => None,
    };
    Ok(data)
}

async fn list_routers(mut top: usize) -> String {
    if top > 10000 {
        top = 10000
    }
    Runtime::instance().extends.router().await.list(top).join("\n")
}

async fn plugin_list() -> Vec<serde_json::Value> {
    let mut plugins = Vec::new();
    for entry in Runtime::instance().plugins.iter() {
        plugins.push(entry.to_json().await);
    }
    plugins
}

async fn plugin_config(name: &str) -> Result<serde_json::Value> {
    Runtime::instance().plugins.get_config(name).await
}
