#![deny(unsafe_code)]

use std::{fs::File, io::BufReader, process, sync::Arc, time::Duration};

#[cfg(not(target_os = "windows"))]
use rustls::crypto::aws_lc_rs as provider;
#[cfg(target_os = "windows")]
use rustls::crypto::ring as provider;
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig};

use rmqtt::anyhow::anyhow;
use rmqtt::broker::{
    v3::control_message as control_message_v3, v3::handshake as handshake_v3, v3::publish as publish_v3,
    v5::control_message as control_message_v5, v5::handshake as handshake_v5, v5::publish as publish_v5,
};
use rmqtt::futures::future::ok;
use rmqtt::ntex::{
    self,
    rt::net::TcpStream,
    server::rustls::Acceptor,
    server::rustls::TlsStream,
    {fn_factory_with_config, fn_service, pipeline_factory},
};
use rmqtt::ntex_mqtt::{
    self,
    v3::Handshake as HandshakeV3,
    v5::Handshake as HandshakeV5,
    {v3, v5, MqttServer},
};
use rmqtt::settings::{listener::Listener, Options, Settings};
use rmqtt::{log, structopt::StructOpt, tokio};
use rmqtt::{logger::logger_init, runtime, MqttError, Result, Runtime, SessionState};

mod ws;

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(dead_code)]
mod plugin {
    include!(concat!(env!("OUT_DIR"), "/plugin.rs"));

    pub(crate) fn default_startups() -> Vec<String> {
        rmqtt::Runtime::instance().settings.plugins.default_startups.clone()
    }
}

#[ntex::main]
async fn main() {
    //init config
    Settings::init(Options::from_args()).expect("settings init failed");

    //init global task executor
    Runtime::init().await.expect("runtime init failed");

    //init log
    let _guard = logger_init().expect("logger init failed");

    let _ = Settings::logs();

    //init scheduler
    runtime::scheduler_init().await.expect("scheduler init failed");

    //start gRPC server
    Runtime::instance().node.start_grpc_server();

    //register plugin
    plugin::registers(plugin::default_startups()).await.expect("register plugin failed");

    //hook, before startup
    Runtime::instance().extends.hook_mgr().await.before_startup().await;

    //tcp
    for (_, listen_cfg) in Runtime::instance().settings.listeners.tcps.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        ntex::rt::spawn(async {
            if let Err(err) = listen(name, listen_cfg).await {
                log::error!("listen mqtt failed: {}", err);
                process::exit(1);
            }
        });
    }

    //tls
    for (_, listen_cfg) in Runtime::instance().settings.listeners.tlss.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        ntex::rt::spawn(async {
            if let Err(err) = listen_tls(name, listen_cfg).await {
                log::error!("listen mqtt tls failed: {}", err);
                process::exit(1);
            }
        });
    }

    //websocket
    for (_, listen_cfg) in Runtime::instance().settings.listeners.wss.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        ntex::rt::spawn(async {
            if let Err(err) = listen_ws(name, listen_cfg).await {
                log::error!("listen websocket failed: {}", err);
                process::exit(1);
            }
        });
    }

    //tls-websocket
    for (_, listen_cfg) in Runtime::instance().settings.listeners.wsss.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        ntex::rt::spawn(async {
            if let Err(err) = listen_wss(name, listen_cfg).await {
                log::error!("listen websocket tls failed: {}", err);
                process::exit(1);
            }
        });
    }

    ntex::rt::signal::ctrl_c().await.expect("signal ctrl c");
    tokio::time::sleep(Duration::from_secs(1)).await;
}

async fn listen(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen(name: &str, listen_cfg: &Listener) -> Result<()> {
        let max_inflight = listen_cfg.max_inflight.get() as usize;
        let handshake_timeout = listen_cfg.handshake_timeout();
        let max_size = listen_cfg.max_packet_size.as_u32();
        ntex::server::Server::build()
            .backlog(listen_cfg.backlog)
            .reuseaddr(listen_cfg.reuseaddr)
            .reuseport(listen_cfg.reuseport)
            .bind(name, listen_cfg.addr, move || {
                MqttServer::new()
                    .v3(v3::MqttServer::new(move |mut handshake: HandshakeV3<TcpStream>| async {
                        let remote_addr = handshake.io().peer_addr()?;
                        let local_addr = handshake.io().local_addr()?;
                        let listen_cfg =
                            Runtime::instance().settings.listeners.tcp(local_addr.port()).ok_or_else(
                                || {
                                    log::error!(
                                        "tcp listener config is not found, local addr is {:?}",
                                        local_addr
                                    );
                                    MqttError::ListenerConfigError
                                },
                            )?;
                        handshake_v3(listen_cfg, handshake, remote_addr, local_addr).await
                    })
                    // .v3(v3::MqttServer::new(handshake_v3)
                    .inflight(max_inflight)
                    .handshake_timeout(handshake_timeout)
                    .max_size(max_size)
                    .publish(fn_factory_with_config(|session: v3::Session<SessionState>| {
                        ok::<_, MqttError>(fn_service(move |req| publish_v3(session.clone(), req)))
                    }))
                    .control(fn_factory_with_config(
                        |session: v3::Session<SessionState>| {
                            ok::<_, MqttError>(fn_service(move |req| {
                                control_message_v3(session.clone(), req)
                            }))
                        },
                    )))
                    .v5(v5::MqttServer::new(move |mut handshake: HandshakeV5<TcpStream>| async {
                        let peer_addr = handshake.io().peer_addr()?;
                        let local_addr = handshake.io().local_addr()?;
                        let listen_cfg =
                            Runtime::instance().settings.listeners.tcp(local_addr.port()).ok_or_else(
                                || {
                                    log::error!(
                                        "tcp listener config is not found, local addr is {:?}",
                                        local_addr
                                    );
                                    MqttError::ListenerConfigError
                                },
                            )?;
                        handshake_v5(listen_cfg, handshake, peer_addr, local_addr).await
                    })
                    //v5::MqttServer::new(handshake_v5)
                    .receive_max(max_inflight as u16)
                    .handshake_timeout(handshake_timeout)
                    .max_size(max_size)
                    // .max_qos(max_qos)
                    //.max_topic_alias(max_topic_alias),
                    .publish(fn_factory_with_config(|session: v5::Session<SessionState>| {
                        ok::<_, MqttError>(fn_service(move |req| publish_v5(session.clone(), req)))
                    }))
                    .control(fn_factory_with_config(
                        |session: v5::Session<SessionState>| {
                            ok::<_, MqttError>(fn_service(move |req| {
                                control_message_v5(session.clone(), req)
                            }))
                        },
                    )))
            })?
            .workers(listen_cfg.workers)
            .maxconn(listen_cfg.max_connections / listen_cfg.workers)
            .run()
            .await?;
        Ok(())
    }

    _listen(&format!("tcp: {}", name), listen_cfg).await.map_err(|e| {
        log::error!("Listen {:?} failed on {}, {:?}", name, listen_cfg.addr, e);
        e
    })
}

async fn listen_tls(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen_tls(name: &str, listen_cfg: &Listener) -> Result<()> {
        let cert_file = &mut BufReader::new(File::open(
            listen_cfg.cert.as_ref().ok_or::<MqttError>("cert is None".into())?,
        )?);
        let key_file = &mut BufReader::new(File::open(
            listen_cfg.key.as_ref().ok_or::<MqttError>("key is None".into())?,
        )?);

        let cert_chain = rustls_pemfile::certs(cert_file).collect::<Result<Vec<_>, _>>()?;
        let key = rustls_pemfile::private_key(key_file)?.ok_or::<MqttError>("key_file is None".into())?;

        let provider = Arc::new(provider::default_provider());
        let client_auth = if listen_cfg.cross_certificate {
            let root_chain = cert_chain.clone();
            let mut client_auth_roots = RootCertStore::empty();
            for root in root_chain {
                client_auth_roots.add(root).map_err(|e| anyhow!(e))?;
            }
            WebPkiClientVerifier::builder_with_provider(client_auth_roots.into(), provider.clone())
                .build()
                .map_err(|e| anyhow!(e))?
        } else {
            WebPkiClientVerifier::no_client_auth()
        };

        let tls_config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| anyhow!(e))?
            .with_client_cert_verifier(client_auth)
            .with_single_cert(cert_chain, key)
            .map_err(|e| anyhow!(format!("bad certs/private key, {}", e)))?;

        let tls_acceptor = Acceptor::new(tls_config);

        let max_inflight = listen_cfg.max_inflight.get() as usize;
        let handshake_timeout = listen_cfg.handshake_timeout();
        let max_size = listen_cfg.max_packet_size.as_u32();
        ntex::server::Server::build()
            .backlog(listen_cfg.backlog)
            .reuseaddr(listen_cfg.reuseaddr)
            .reuseport(listen_cfg.reuseport)
            .bind(name, listen_cfg.addr, move || {
                pipeline_factory(tls_acceptor.clone())
                    .map_err(|e| ntex_mqtt::MqttError::Service(MqttError::from(e)))
                    .and_then(
                        MqttServer::new()
                            .v3(v3::MqttServer::new(
                                move |mut handshake: HandshakeV3<TlsStream<TcpStream>>| async {
                                    let (io, _) = handshake.io().get_ref();
                                    let peer_addr = io.peer_addr()?;
                                    let local_addr = io.local_addr()?;
                                    let listen_cfg = Runtime::instance()
                                        .settings
                                        .listeners
                                        .tls(local_addr.port())
                                        .ok_or_else(|| {
                                            log::error!(
                                                "tls listener config is not found, local addr is {:?}",
                                                local_addr
                                            );
                                            MqttError::ListenerConfigError
                                        })?;

                                    handshake_v3(listen_cfg, handshake, peer_addr, local_addr).await
                                },
                            )
                            //.v3(v3::MqttServer::new(handshake_v3)
                            .inflight(max_inflight)
                            .handshake_timeout(handshake_timeout)
                            .max_size(max_size)
                            .publish(fn_factory_with_config(|session: v3::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| publish_v3(session.clone(), req)))
                            }))
                            .control(fn_factory_with_config(
                                |session: v3::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        control_message_v3(session.clone(), req)
                                    }))
                                },
                            )))
                            .v5(
                                //v5::MqttServer::new(handshake_v5)
                                v5::MqttServer::new(
                                    move |mut handshake: HandshakeV5<TlsStream<TcpStream>>| async {
                                        let (io, _) = handshake.io().get_ref();
                                        let peer_addr = io.peer_addr()?;
                                        let local_addr = io.local_addr()?;
                                        let listen_cfg = Runtime::instance()
                                            .settings
                                            .listeners
                                            .tls(local_addr.port())
                                            .ok_or_else(|| {
                                                log::error!(
                                                    "tls listener config is not found, local addr is {:?}",
                                                    local_addr
                                                );
                                                MqttError::ListenerConfigError
                                            })?;
                                        handshake_v5(listen_cfg, handshake, peer_addr, local_addr).await
                                    },
                                )
                                .receive_max(max_inflight as u16)
                                .handshake_timeout(handshake_timeout)
                                .max_size(max_size)
                                // .max_qos(max_qos)
                                //.max_topic_alias(max_topic_alias)
                                .publish(fn_factory_with_config(|session: v5::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        publish_v5(session.clone(), req)
                                    }))
                                }))
                                .control(fn_factory_with_config(
                                    |session: v5::Session<SessionState>| {
                                        ok::<_, MqttError>(fn_service(move |req| {
                                            control_message_v5(session.clone(), req)
                                        }))
                                    },
                                )),
                            ),
                    )
            })?
            .workers(listen_cfg.workers)
            .maxconn(listen_cfg.max_connections / listen_cfg.workers)
            .run()
            .await?;
        Ok(())
    }

    _listen_tls(&format!("tls: {}", name), listen_cfg).await.map_err(|e| {
        log::error!(
            "Listen_tls {:?} failed on {}, cert: {:?}, key: {:?}, {:?}",
            name,
            listen_cfg.addr,
            listen_cfg.cert,
            listen_cfg.key,
            e
        );
        e
    })
}

async fn listen_ws(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen_ws(name: &str, listen_cfg: &Listener) -> Result<()> {
        let max_inflight = listen_cfg.max_inflight.get() as usize;
        let handshake_timeout = listen_cfg.handshake_timeout();
        let max_size = listen_cfg.max_packet_size.as_u32();
        ntex::server::Server::build()
            .backlog(listen_cfg.backlog)
            .reuseaddr(listen_cfg.reuseaddr)
            .reuseport(listen_cfg.reuseport)
            .bind(name, listen_cfg.addr, move || {
                pipeline_factory(ws::WSServer::new(Duration::from_secs(handshake_timeout as u64))).and_then(
                    MqttServer::new()
                        .v3(v3::MqttServer::new(
                            move |mut handshake: HandshakeV3<ws::WsStream<TcpStream>>| async {
                                let io = handshake.io().get_ref();
                                let remote_addr = io.peer_addr()?;
                                let local_addr = io.local_addr()?;
                                let listen_cfg =
                                    Runtime::instance().settings.listeners.ws(local_addr.port()).ok_or_else(
                                        || {
                                            log::error!(
                                                "ws listener config is not found, local addr is {:?}",
                                                local_addr
                                            );
                                            MqttError::ListenerConfigError
                                        },
                                    )?;
                                handshake_v3(listen_cfg, handshake, remote_addr, local_addr).await
                            },
                        )
                        .inflight(max_inflight)
                        .handshake_timeout(handshake_timeout)
                        .max_size(max_size)
                        .publish(fn_factory_with_config(|session: v3::Session<SessionState>| {
                            ok::<_, MqttError>(fn_service(move |req| publish_v3(session.clone(), req)))
                        }))
                        .control(fn_factory_with_config(
                            |session: v3::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    control_message_v3(session.clone(), req)
                                }))
                            },
                        )))
                        .v5(v5::MqttServer::new(
                            move |mut handshake: HandshakeV5<ws::WsStream<TcpStream>>| async {
                                let io = handshake.io().get_ref();
                                let remote_addr = io.peer_addr()?;
                                let local_addr = io.local_addr()?;
                                let listen_cfg =
                                    Runtime::instance().settings.listeners.ws(local_addr.port()).ok_or_else(
                                        || {
                                            log::error!(
                                                "ws listener config is not found, local addr is {:?}",
                                                local_addr
                                            );
                                            MqttError::ListenerConfigError
                                        },
                                    )?;
                                handshake_v5(listen_cfg, handshake, remote_addr, local_addr).await
                            },
                        )
                        .receive_max(max_inflight as u16)
                        .handshake_timeout(handshake_timeout)
                        .max_size(max_size)
                        // .max_qos(max_qos)
                        //.max_topic_alias(max_topic_alias),
                        .publish(fn_factory_with_config(|session: v5::Session<SessionState>| {
                            ok::<_, MqttError>(fn_service(move |req| publish_v5(session.clone(), req)))
                        }))
                        .control(fn_factory_with_config(
                            |session: v5::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    control_message_v5(session.clone(), req)
                                }))
                            },
                        ))),
                )
            })?
            .workers(listen_cfg.workers)
            .maxconn(listen_cfg.max_connections / listen_cfg.workers)
            .run()
            .await?;
        Ok(())
    }

    _listen_ws(&format!("ws: {}", name), listen_cfg).await.map_err(|e| {
        log::error!("Listen {:?} failed on {}, {:?}", name, listen_cfg.addr, e);
        e
    })
}

async fn listen_wss(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen_wss(name: &str, listen_cfg: &Listener) -> Result<()> {
        let cert_file = &mut BufReader::new(File::open(
            listen_cfg.cert.as_ref().ok_or::<MqttError>("cert is None".into())?,
        )?);
        let key_file = &mut BufReader::new(File::open(
            listen_cfg.key.as_ref().ok_or::<MqttError>("key is None".into())?,
        )?);

        let cert_chain = rustls_pemfile::certs(cert_file).collect::<Result<Vec<_>, _>>()?;
        let key = rustls_pemfile::private_key(key_file)?.ok_or::<MqttError>("key_file is None".into())?;

        let provider = Arc::new(provider::default_provider());
        let client_auth = if listen_cfg.cross_certificate {
            let root_chain = cert_chain.clone();
            let mut client_auth_roots = RootCertStore::empty();
            for root in root_chain {
                client_auth_roots.add(root).map_err(|e| anyhow!(e))?;
            }
            WebPkiClientVerifier::builder_with_provider(client_auth_roots.into(), provider.clone())
                .build()
                .map_err(|e| anyhow!(e))?
        } else {
            WebPkiClientVerifier::no_client_auth()
        };

        let tls_config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| anyhow!(e))?
            .with_client_cert_verifier(client_auth)
            .with_single_cert(cert_chain, key)
            .map_err(|e| anyhow!(format!("bad certs/private key, {}", e)))?;

        let tls_acceptor = Acceptor::new(tls_config);

        let max_inflight = listen_cfg.max_inflight.get() as usize;
        let handshake_timeout = listen_cfg.handshake_timeout();
        let max_size = listen_cfg.max_packet_size.as_u32();
        ntex::server::Server::build()
            .backlog(listen_cfg.backlog)
            .reuseaddr(listen_cfg.reuseaddr)
            .reuseport(listen_cfg.reuseport)
            .bind(name, listen_cfg.addr, move || {
                pipeline_factory(tls_acceptor.clone())
                    .map_err(|e| ntex_mqtt::MqttError::Service(MqttError::from(e)))
                    .and_then(ws::WSServer::new(Duration::from_secs(handshake_timeout as u64)))
                    .and_then(
                        MqttServer::new()
                            .v3(v3::MqttServer::new(
                                move |mut handshake: HandshakeV3<ws::WsStream<TlsStream<TcpStream>>>| async {
                                    let (io, _) = handshake.io().get_ref().get_ref();
                                    let peer_addr = io.peer_addr()?;
                                    let local_addr = io.local_addr()?;
                                    let listen_cfg = Runtime::instance()
                                        .settings
                                        .listeners
                                        .wss(local_addr.port())
                                        .ok_or_else(|| {
                                            log::error!(
                                                "wss listener config is not found, local addr is {:?}",
                                                local_addr
                                            );
                                            MqttError::ListenerConfigError
                                        })?;

                                    handshake_v3(listen_cfg, handshake, peer_addr, local_addr).await
                                },
                            )
                            .inflight(max_inflight)
                            .handshake_timeout(handshake_timeout)
                            .max_size(max_size)
                            .publish(fn_factory_with_config(|session: v3::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| publish_v3(session.clone(), req)))
                            }))
                            .control(fn_factory_with_config(
                                |session: v3::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        control_message_v3(session.clone(), req)
                                    }))
                                },
                            )))
                            .v5(v5::MqttServer::new(
                                move |mut handshake: HandshakeV5<ws::WsStream<TlsStream<TcpStream>>>| async {
                                    let (io, _) = handshake.io().get_ref().get_ref();
                                    let peer_addr = io.peer_addr()?;
                                    let local_addr = io.local_addr()?;
                                    let listen_cfg = Runtime::instance()
                                        .settings
                                        .listeners
                                        .wss(local_addr.port())
                                        .ok_or_else(|| {
                                            log::error!(
                                                "wss listener config is not found, local addr is {:?}",
                                                local_addr
                                            );
                                            MqttError::ListenerConfigError
                                        })?;
                                    handshake_v5(listen_cfg, handshake, peer_addr, local_addr).await
                                },
                            )
                            .receive_max(max_inflight as u16)
                            .handshake_timeout(handshake_timeout)
                            .max_size(max_size)
                            // .max_qos(max_qos)
                            //.max_topic_alias(max_topic_alias)
                            .publish(fn_factory_with_config(|session: v5::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| publish_v5(session.clone(), req)))
                            }))
                            .control(fn_factory_with_config(
                                |session: v5::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        control_message_v5(session.clone(), req)
                                    }))
                                },
                            ))),
                    )
            })?
            .workers(listen_cfg.workers)
            .maxconn(listen_cfg.max_connections / listen_cfg.workers)
            .run()
            .await?;
        Ok(())
    }

    _listen_wss(&format!("wss: {}", name), listen_cfg).await.map_err(|e| {
        log::error!(
            "listen_wss {:?} failed on {}, cert: {:?}, key: {:?}, {:?}",
            name,
            listen_cfg.addr,
            listen_cfg.cert,
            listen_cfg.key,
            e
        );
        e
    })
}
