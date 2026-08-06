//! Binary entry point for the RMQTT broker.
//!
//! Parses CLI arguments, loads configuration, initializes the logger, creates
//! the server context, registers plugins, binds listeners (TCP, TLS, WebSocket,
//! WSS, QUIC), and starts the MQTT server. Handles OS signal-based shutdown.

#![deny(unsafe_code)]

use anyhow::anyhow;
use std::time::Duration;

use clap::Parser;

use rmqtt::args::CommandArgs;
use rmqtt::context::{
    CircuitBreakerConfig, CountBasedWindowConfig, ServerContext, TimeBasedWindowConfig, WindowConfig,
};
use rmqtt::net::{tls_provider, Builder};
use rmqtt::node::Node;
use rmqtt::server::MqttServer;
use rmqtt::Result;
use rmqtt_conf::{listener::Listener, Options, Settings};

mod logger;

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(dead_code)]
mod plugin {
    include!(concat!(env!("OUT_DIR"), "/plugin.rs"));
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Err(e) = run().await {
        log::error!("{e}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    let opts = Options::parse();
    if opts.version {
        println!("{} rustc/{}", Node::version(), Node::rustc_version());
        return Ok(());
    }

    //init config
    let conf = Settings::init(Options::parse()).expect("settings init failed");

    //rustls crypto install default
    tls_provider::default_provider()
        .install_default()
        .expect("Failed to install the default Rustls crypto backend because it is already installed.");

    //init log
    logger::logger_init(&conf.log).expect("logger init failed");

    //node info
    let node = Node::new(
        conf.node.id,
        conf.node.busy.loadavg,
        conf.node.busy.cpuloadavg,
        conf.node.busy.update_interval,
    );

    //print version
    log::info!("Starting rmqtt (version={}, rustc={})", Node::version(), Node::rustc_version());

    let _ = Settings::logs();

    // Build circuit-breaker config from global settings.
    let cb_config = {
        let cb = &Settings::instance().circuit_breaker;
        let window = match cb.sliding_window_type.as_str() {
            "CountBased" => WindowConfig::CountBased(CountBasedWindowConfig {
                sliding_window_size: cb.sliding_window_size,
            }),
            _ => WindowConfig::TimeBased(TimeBasedWindowConfig {
                sliding_window_duration: cb.sliding_window_duration,
                sliding_window_size: cb.sliding_window_size,
            }),
        };
        CircuitBreakerConfig {
            failure_rate_threshold: cb.failure_rate_threshold,
            window,
            minimum_number_of_calls: cb.minimum_number_of_calls,
            wait_duration_in_open: cb.wait_duration_in_open,
            slow_call_duration_threshold: cb.slow_call_duration_threshold,
            slow_call_rate_threshold: cb.slow_call_rate_threshold,
            name: "grpc".into(),
        }
    };

    //init ServerContext
    let scx = ServerContext::new()
        .args(config_args(conf))
        .node(node)
        .task_exec_workers(conf.task.exec_workers)
        .task_exec_queue_max(conf.task.exec_queue_max)
        .busy_check_enable(conf.node.busy.check_enable)
        .busy_handshaking_limit(conf.node.busy.handshaking)
        .mqtt_delayed_publish_max(conf.mqtt.delayed_publish_max)
        .mqtt_delayed_publish_immediate(conf.mqtt.delayed_publish_immediate)
        .mqtt_max_sessions(conf.mqtt.max_sessions)
        .plugins_config_dir(conf.plugins.dir.as_str())
        .circuit_breaker_config(cb_config)
        .build()
        .await;

    //start gRPC server
    scx.node.start_grpc_server(scx.clone(), conf.rpc.server_addr, conf.rpc.reuseaddr, conf.rpc.reuseport);

    //register plugin
    plugin::registers(
        &scx,
        conf.plugins.default_startups.clone(),
        conf.plugins.disabled_default_startups.clone(),
    )
    .await
    .expect("register plugin failed");

    let mut builder = MqttServer::new(scx);

    //tcp
    for listen_cfg in Settings::instance().listeners.tcps.values() {
        let listener = match config_builder(listen_cfg).bind() {
            Ok(l) => l.tcp()?,
            Err(e) => {
                return Err(anyhow!("Failed to bind TCP listener on {}: {}", listen_cfg.addr, e));
            }
        };
        builder = builder.listener(listener);
    }

    //tls
    for listen_cfg in Settings::instance().listeners.tlss.values() {
        let listener = match config_builder(listen_cfg).bind() {
            Ok(l) => l.tls()?,
            Err(e) => {
                return Err(anyhow!("Failed to bind TLS listener on {}: {}", listen_cfg.addr, e));
            }
        };
        builder = builder.listener(listener);
    }

    //websocket
    for listen_cfg in Settings::instance().listeners.wss.values() {
        let listener = match config_builder(listen_cfg).bind() {
            Ok(l) => l.ws()?,
            Err(e) => {
                return Err(anyhow!("Failed to bind WebSocket listener on {}: {}", listen_cfg.addr, e));
            }
        };
        builder = builder.listener(listener);
    }

    //tls-websocket
    for listen_cfg in Settings::instance().listeners.wsss.values() {
        let listener = match config_builder(listen_cfg).bind() {
            Ok(l) => l.wss()?,
            Err(e) => {
                return Err(anyhow!("Failed to bind WSS listener on {}: {}", listen_cfg.addr, e));
            }
        };
        builder = builder.listener(listener);
    }

    //MQTT over QUIC
    for listen_cfg in Settings::instance().listeners.quics.values() {
        let listener = match config_builder(listen_cfg).bind_quic() {
            Ok(l) => l,
            Err(e) => {
                return Err(anyhow!("Failed to bind QUIC listener on {}: {}", listen_cfg.addr, e));
            }
        };
        builder = builder.listener(listener);
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
    //log::info!("Performing cleanup..."); @TODO ... Hook when exiting

    tokio::time::sleep(Duration::from_millis(100)).await;
    std::process::exit(0);
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
        .max_session_expiry_interval(cfg.max_session_expiry_interval)
        .message_retry_interval(cfg.message_retry_interval)
        .message_expiry_interval(cfg.message_expiry_interval)
        .max_subscriptions(cfg.max_subscriptions)
        .max_topic_aliases(cfg.max_topic_aliases)
        .tls_cross_certificate(cfg.cross_certificate)
        .tls_cert(cfg.cert.clone())
        .tls_key(cfg.key.clone())
        .tls_client_ca_certs(cfg.client_ca_certs.clone())
        .limit_subscription(cfg.limit_subscription)
        .delayed_publish(cfg.delayed_publish)
        .proxy_protocol(cfg.proxy_protocol)
        .proxy_protocol_timeout(cfg.proxy_protocol_timeout)
        .cert_cn_as_username(cfg.cert_cn_as_username)
        .cert_subject_dn_as_username(cfg.cert_subject_dn_as_username)
        .collect_cert_info(cfg.collect_cert_info)
        .idle_timeout(cfg.idle_timeout)
}

fn config_args(cfg: &Settings) -> CommandArgs {
    CommandArgs {
        node_id: cfg.opts.node_id,
        plugins_default_startups: cfg.opts.plugins_default_startups.clone(),
        node_grpc_addrs: cfg.opts.node_grpc_addrs.clone(),
        raft_peer_addrs: cfg.opts.raft_peer_addrs.clone(),
        raft_leader_id: cfg.opts.raft_leader_id,
    }
}
