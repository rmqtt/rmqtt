#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::net::SocketAddr;

use salvo::affix;
use salvo::http::header::{HeaderValue, CONTENT_TYPE};
use salvo::prelude::*;
//use salvo::logging::Logger;  ??
use salvo::serve_static::StaticDir;
use salvo::session::{CookieStore, Session, SessionDepotExt, SessionHandler};
use std::env;

use std::sync::Arc;


use config::PluginConfig;
use async_trait::async_trait;
use rmqtt::{
    anyhow, RwLock,
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    tokio::sync::oneshot,
    Result, Runtime,
};
type ShutdownTX = oneshot::Sender<()>;
type PluginConfigType = Arc<RwLock<PluginConfig>>;

use tracing_subscriber::{
    fmt,
    layer::SubscriberExt,
};
use tracing_appender::{
    non_blocking::{
        WorkerGuard,
    },
};
    
use tracing::debug;

mod config;
//use super::types::{
//    ClientSearchParams, Message, MessageReply, PublishParams, SubscribeParams, UnsubscribeParams,
//};
//use super::PluginConfigType;
//use super::{clients, plugin, subs};

#[handler]
async fn hello(depot: &mut Depot) -> &'static str {
    let sess = depot.session();
    println!("{:?}", sess);
    "Hello World"
}

//#[handler]
async fn srv_static() -> StaticDir {
    StaticDir::new([
        "dist",
    ])
      .with_defaults("index.html")
      .with_listing(true)
}

#[handler]
pub async fn login(req: &mut Request, depot: &mut Depot, res: &mut Response) {
    let sess = depot.session();
    println!("{:?}", sess);
    if req.method() == salvo::http::Method::POST {
        let mut session = Session::new();
        session
            .insert("username", req.form::<String>("username").await.unwrap())
            .unwrap();
        depot.set_session(session);
        res.render(Redirect::other("/"));
    } else {
        res.render(Text::Html(LOGIN_HTML));
    }
}

#[handler]
pub async fn logout(depot: &mut Depot, res: &mut Response) {
    if let Some(session) = depot.session_mut() {
        session.remove("username");
    }
    res.render(Redirect::other("/"));
}

fn init_tracing() -> WorkerGuard {
    let file_appender = tracing_appender::rolling::hourly("./logs", "log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    tracing::subscriber::set_global_default(
        fmt::Subscriber::builder()
            // subscriber configuration
            .with_env_filter("server")
            .with_max_level(tracing::Level::DEBUG)
            .finish()
            // add additional writers
            .with(fmt::Layer::default().with_writer(file_writer))
    ).expect("Unable to set global tracing subscriber");
    debug!("Tracing initialized.");
    guard
}

fn route(cfg: PluginConfigType) -> Router {
    println!("{:?}", env::current_dir().expect("SHT").display());
    
    let _guard = init_tracing();

    let session_handler = SessionHandler::builder(
        CookieStore::new(),
        b"secretabsecretabsecretabsecretabsecretabsecretabsecretabsecretab",
    )
    .build()
    .unwrap();
    Router::new().hoop(Logger).hoop(session_handler)
        .push(Router::with_path("/app").get(hello))
        .push(Router::with_path("login").get(login).post(login))
        .push(Router::with_path("logout").get(logout))
        .push(Router::with_path("<**path>").get(
            StaticDir::new([
                "dist",
            ])
              .with_defaults("index.html")
              .with_listing(true)
        ))
   

}

pub(crate) async fn listen_and_serve(
    laddr: SocketAddr,
    cfg: PluginConfigType,
    rx: oneshot::Receiver<()>,
) -> Result<()> {
    log::info!("HTTP WEBListening on {}", laddr);
    Server::new(TcpListener::bind(laddr))
        .try_serve_with_graceful_shutdown(route(cfg), async {
            rx.await.ok();
        })
        .await
        .map_err(anyhow::Error::new)?;
    Ok(())
}


#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    descr: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(async move {
                HttpWeb::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct HttpWeb {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: PluginConfigType,
    shutdown_tx: Option<ShutdownTX>,
}

impl HttpWeb {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} HttpWebPlugin cfg: {:?}", name, cfg.read());
        let register = runtime.extends.hook_mgr().await.register();
        let shutdown_tx = Some(Self::start(runtime, cfg.clone()));
        Ok(Self { runtime, name: name.into(), descr: descr.into(), register, cfg, shutdown_tx })
    }
    #[inline]
    fn start(_runtime: &'static Runtime, cfg: PluginConfigType) -> ShutdownTX {
        let (shutdown_tx, shutdown_rx): (oneshot::Sender<()>, oneshot::Receiver<()>) = oneshot::channel();

        let _child = std::thread::Builder::new().name("http-api".to_string()).spawn(move || {
            let cfg1 = cfg.clone();
            let runner = async move {
                let laddr = cfg1.read().http_laddr;
                if let Err(e) = listen_and_serve(laddr, cfg1, shutdown_rx).await {
                    log::error!("{:?}", e);
                }
            };

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .worker_threads(cfg.read().workers)
                .thread_name("http-api-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            rt.block_on(runner);
            log::info!("Exit HTTP API Server, ..., http://{:?}", cfg.read().http_laddr);
        });
        shutdown_tx
    }

}

#[async_trait]
impl Plugin for HttpWeb {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::debug!("{} init", self.name);
        self.register.add(Type::ClientConnack, Box::new(HookHandler::new())).await;
        self.register.add(Type::ClientSubscribe, Box::new(HookHandler::new())).await;
        self.register.add(Type::ClientUnsubscribe, Box::new(HookHandler::new())).await;
        self.register.add(Type::MessageDelivered, Box::new(HookHandler::new())).await;
        self.register.add(Type::MessagePublish, Box::new(HookHandler::new())).await;
        self.register.add_priority(Type::ClientSubscribeCheckAcl, 10, Box::new(HookHandler::new())).await;
        self.register.add_priority(Type::GrpcMessageReceived, 10, Box::new(HookHandler::new())).await;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        self.register.stop().await;
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}

struct HookHandler {}

impl HookHandler {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnack(connect_info, r) => {
                log::debug!("client connack, {:?}, {:?}", connect_info, r);
            }
            Parameter::ClientSubscribe(_session, c, subscribe) => {
                log::debug!("{:?} client subscribe, {:?}", c.id, subscribe);
                //let mut topic_filter = subscribe.topic_filter.clone();
                //topic_filter.insert(0, Level::Normal("PPP".into()));
                //return (true, Some(HookResult::TopicFilter(Some(topic_filter))))
            }
            Parameter::ClientUnsubscribe(_session, c, unsubscribe) => {
                log::debug!("{:?} client unsubscribe, {:?}", c.id, unsubscribe);
                //let mut topic_filter = (*unsubscribe).clone();
                //topic_filter.insert(0, Level::Normal("PPP".into()));
                //return (true, Some(HookResult::TopicFilter(Some(topic_filter))))
            }
            Parameter::MessagePublish(_session, c, publish) => {
                log::debug!("{:?} message publish, {:?}", c.id, publish);
            }
            Parameter::MessageDelivered(_session, c, from, _publish) => {
                log::debug!("{:?} MessageDelivered, {:?}", c.id, from);
            }
            Parameter::ClientSubscribeCheckAcl(_s, _c, subscribe) => {
                log::debug!("{:?} ClientSubscribeCheckAcl, {:?}", _c.id, subscribe);
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

static LOGIN_HTML: &str = r#"<!DOCTYPE html>
<html>
    <head>
        <title>Login</title>
    </head>
    <body>
        <form action="/login" method="post">
            <h1>Login</h1>
            <input type="text" name="username" />
            <button type="submit" id="submit">Submit</button>
        </form>
    </body>
</html>
"#;