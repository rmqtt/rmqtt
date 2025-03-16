#![deny(unsafe_code)]

use std::time::Duration;

use structopt::StructOpt;

// use rmqtt::context::ServerContext;
#[cfg(not(target_os = "windows"))]
use rustls::crypto::aws_lc_rs as provider;
#[cfg(target_os = "windows")]
use rustls::crypto::ring as provider;

use rmqtt::conf::{listener::Listener, Options, Settings};
use rmqtt::context::ServerContext;
use rmqtt::logger::logger_init;
use rmqtt::net::Builder;
use rmqtt::node::Node;
use rmqtt::server::MqttServer;
use rmqtt::Result;

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(dead_code)]
mod plugin {
    include!(concat!(env!("OUT_DIR"), "/plugin.rs"));

    // pub(crate) fn default_startups(cfg: &rmqtt::conf::Settings) -> Vec<String> {
    //     cfg.plugins.default_startups.clone()
    // }
}

#[tokio::main]
async fn main() -> Result<()> {
    //init config
    let conf = Settings::init(Options::from_args()).expect("settings init failed");

    //rustls crypto install default
    provider::default_provider()
        .install_default()
        .expect("Failed to install the default Rustls crypto backend because it is already installed.");

    //init log
    let (_guard, logger) = logger_init(&conf.log).expect("logger init failed");

    let _ = Settings::logs();

    //node info
    let node = Node::new(
        conf.node.id,
        conf.node.busy.loadavg,
        conf.node.busy.cpuloadavg,
        conf.node.busy.update_interval,
    );

    //init ServerContext
    let scx = ServerContext::new(conf.clone(), logger, node).config().await;

    // //init scheduler
    // ServerContext::scheduler_init().await.expect("scheduler init failed");

    //start gRPC server
    scx.node.start_grpc_server(
        scx.clone(),
        conf.rpc.server_addr,
        conf.rpc.server_workers,
        conf.rpc.reuseaddr,
        conf.rpc.reuseport,
    );

    //register plugin
    plugin::registers(&scx, conf.plugins.default_startups.clone()).await.expect("register plugin failed");

    //hook, before startup
    scx.extends.hook_mgr().before_startup().await;

    let mut builder = MqttServer::new(scx);

    //tcp
    for (_, listen_cfg) in Settings::instance().listeners.tcps.iter() {
        builder = builder.listener(config_builder(listen_cfg).bind()?.tcp()?);
    }

    //tls
    for (_, listen_cfg) in Settings::instance().listeners.tlss.iter() {
        builder = builder.listener(config_builder(listen_cfg).bind()?.tls()?);
    }

    //websocket
    for (_, listen_cfg) in Settings::instance().listeners.wss.iter() {
        builder = builder.listener(config_builder(listen_cfg).bind()?.ws()?);
    }

    //tls-websocket
    for (_, listen_cfg) in Settings::instance().listeners.wsss.iter() {
        builder = builder.listener(config_builder(listen_cfg).bind()?.wss()?);
    }

    builder.build().start();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    // Listen for Ctrl+C
    tokio::spawn(async move {
        #[cfg(target_os = "windows")]
        tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
        #[cfg(not(target_os = "windows"))]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut term_signal = signal(SignalKind::terminate()).expect("Failed to create signal handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term_signal.recv() => {}
            }
        }
        shutdown_tx.send(()).expect("Failed to send the shutdown command");
    });

    shutdown_rx.await.expect("Failed to receive the shutdown command");
    log::info!("Performing cleanup...");

    tokio::time::sleep(Duration::from_secs(1)).await;
    Ok(())
}

fn config_builder(cfg: &Listener) -> Builder {
    Builder::new()
        .name(cfg.name.as_str())
        .laddr(cfg.addr)
        .max_connections(cfg.max_connections)
        .max_handshaking_limit(cfg.max_handshaking_limit)
        .max_packet_size(cfg.max_packet_size.as_u32())
        .backlog(cfg.backlog)
        .nodelay(cfg.nodelay)
        .reuseaddr(cfg.reuseaddr)
        .reuseport(cfg.reuseport)
        .allow_anonymous(cfg.allow_anonymous)
        .min_keepalive(cfg.min_keepalive)
        .max_keepalive(cfg.max_keepalive)
        .allow_zero_keepalive(cfg.allow_zero_keepalive)
        .keepalive_backoff(cfg.keepalive_backoff)
        .max_inflight(cfg.max_inflight)
        .handshake_timeout(cfg.handshake_timeout)
        .max_mqueue_len(cfg.max_mqueue_len)
        .mqueue_rate_limit(cfg.mqueue_rate_limit.0, cfg.mqueue_rate_limit.1)
        .max_clientid_len(cfg.max_clientid_len)
        .max_qos_allowed(cfg.max_qos_allowed)
        .max_topic_levels(cfg.max_topic_levels)
        .session_expiry_interval(cfg.session_expiry_interval)
        .message_retry_interval(cfg.message_retry_interval)
        .message_expiry_interval(cfg.message_expiry_interval)
        .max_subscriptions(cfg.max_subscriptions)
        .shared_subscription(cfg.shared_subscription)
        .max_topic_aliases(cfg.max_topic_aliases)
        .tls_cross_certificate(cfg.cross_certificate)
        .tls_cert(cfg.cert.clone())
        .tls_key(cfg.key.clone())
        .limit_subscription(cfg.limit_subscription)
        .delayed_publish(cfg.delayed_publish)
}
