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

    //router topic list, /router/topics/:top
    let router_topics = root()
        .and(warp::path!("router" / "topics" / usize))
        .and(warp::path::end())
        .and_then(|top: usize| async move { with_reply_string(list_router_topics(top).await) })
        .or(root()
            .and(warp::path!("router" / "topics"))
            .and(warp::path::end())
            .and_then(|| async move { with_reply_string(list_router_topics(10000).await) }));

    //router subscription relation list, /router/relations/:top
    let router_relations = root()
        .and(warp::path!("router" / "relations" / usize))
        .and(warp::path::end())
        .and_then(|top: usize| async move { with_reply_json(list_router_relations(top).await) })
        .or(root()
            .and(warp::path!("router" / "relations"))
            .and(warp::path::end())
            .and_then(|| async move { with_reply_json(list_router_relations(10000).await) }));

    // //router shared subscribes list, /router/shared/:top
    // let router_shared_subscribes = root()
    //     .and(warp::path!("router" / "shared_subscribes" / usize))
    //     .and(warp::path::end())
    //     .and_then(|top: usize| async move { with_reply_json(list_router_shared_subscribes(top).await) })
    //     .or(root()
    //         .and(warp::path!("router" / "shared_subscribes"))
    //         .and(warp::path::end())
    //         .and_then(|| async move { with_reply_json(list_router_shared_subscribes(10000).await) }));

    //--plugin--------------------------------------------------------------------------
    //plugin list, /plugin/list
    let plugin_list = root()
        .and(warp::path!("plugin" / "list"))
        .and(warp::path::end())
        .and_then(|| async move { with_reply_json(plugin_list().await) });

    //plugin config, /plugin/:plugin/config
    let plugin_config = root()
        .and(warp::path!("plugin" / String / "config"))
        .and(warp::path::end())
        .and_then(|name: String| async move {
            match plugin_config(&name).await {
                Ok(cfg) => with_reply_json(cfg),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        });

    //start_plugin, "/plugin/:plugin/start", PUT
    let start_plugin = root().and(warp::path!("plugin" / String / "start")).and(warp::path::end()).and_then(
        |name: String| async move {
            match start_plugin(&name).await {
                Ok(_) => with_reply_string("ok"),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        },
    );

    //stop_plugin, "/plugin/:plugin/stop", PUT
    let stop_plugin = root().and(warp::path!("plugin" / String / "stop")).and(warp::path::end()).and_then(
        |name: String| async move {
            match stop_plugin(&name).await {
                Ok(cfg) => with_reply_json(cfg),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        },
    );

    //stop_plugin, "/plugin/:plugin/send", POST/GET
    let plugin_send = root()
        .and(warp::path!("plugin" / String / "send"))
        .and(warp::query::<serde_json::Value>())
        .and(warp::path::end())
        .and_then(|name: String, msg: serde_json::Value| async move {
            match plugin_send(&name, msg).await {
                Ok(cfg) => with_reply_json(cfg),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        });

    //plugin_reload_config, "/plugin/:plugin/config/reload", PUT
    let plugin_reload_config = root()
        .and(warp::path!("plugin" / String / "config" / "reload"))
        .and(warp::path::end())
        .and_then(|name: String| async move {
            match plugin_reload_config(&name).await {
                Ok(()) => with_reply_string("ok"),
                Err(e) => with_reply_status(warp::http::StatusCode::NOT_FOUND, Some(format!("{:?}", e))),
            }
        });

    //---------------------------------------------------------------------------------

    let get_apis = warp::get().and(
        version
            .or(status)
            .or(session)
            .or(random_session)
            .or(router_topics)
            .or(router_relations)
            // .or(router_shared_subscribes)
            .or(plugin_list)
            .or(plugin_config)
            .or(plugin_send),
    );
    let put_apis = warp::put().and(start_plugin.or(stop_plugin).or(plugin_reload_config));
    let post_apis = warp::post().and(plugin_send);

    let routes = get_apis.or(put_apis).or(post_apis);
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
        "all_sessions": shared.all_sessions().await,
        "all_clients": shared.all_clients().await,
        "sessions": shared.sessions().await,
        "clients": shared.clients().await,
        "active_grpc_requests": active_grpc_requests(),
        "shared_subscription_supported": Runtime::instance().extends.shared_subscription().await.is_supported()
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

async fn list_router_topics(mut top: usize) -> String {
    if top > 10000 {
        top = 10000
    }
    Runtime::instance().extends.router().await.list_topics(top).await.join("\n")
}

async fn list_router_relations(mut top: usize) -> Vec<serde_json::Value> {
    if top > 10000 {
        top = 10000
    }
    Runtime::instance().extends.router().await.list_relations(top).await
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

async fn start_plugin(name: &str) -> Result<()> {
    Runtime::instance().plugins.start(name).await
}

async fn stop_plugin(name: &str) -> Result<bool> {
    Runtime::instance().plugins.stop(name).await
}

async fn plugin_send(name: &str, msg: serde_json::Value) -> Result<serde_json::Value> {
    Runtime::instance().plugins.send(name, msg).await
}

async fn plugin_reload_config(name: &str) -> Result<()> {
    Runtime::instance().plugins.load_config(name).await
}
