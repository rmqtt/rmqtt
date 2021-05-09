mod inner_api;

use anyhow::Result;
use futures::future::ok;
use ntex::rt::net::TcpStream;
use ntex::server::rustls::Acceptor;
use ntex::server::rustls::TlsStream;
use ntex::{fn_factory_with_config, fn_service, pipeline_factory};
use ntex_mqtt::v3::Handshake as HandshakeV3;
use ntex_mqtt::v5::Handshake as HandshakeV5;
use ntex_mqtt::{v3, v5, MqttServer};
use rustls::internal::pemfile::{certs, rsa_private_keys};
use rustls::{NoClientAuth, ServerConfig};
use std::{fs::File, io::BufReader};

use rmqtt::broker::{
    v3::control_message as control_message_v3, v3::handshake as handshake_v3,
    v3::publish as publish_v3, v5::control_message as control_message_v5,
    v5::handshake as handshake_v5, v5::publish as publish_v5,
};
use rmqtt::settings::listener::Listener;
use rmqtt::{logger::logger_init, MqttError, Runtime, SessionState};

#[allow(dead_code)]
mod plugin {
    include!(concat!(env!("OUT_DIR"), "/plugin.rs"));

    pub(crate) async fn default_startups() -> rmqtt::Result<()> {
        for name in rmqtt::Runtime::instance()
            .settings
            .plugins
            .default_startups
            .iter()
        {
            rmqtt::Runtime::instance()
                .plugins
                .start(name.as_str())
                .await?;
        }
        Ok(())
    }
}

#[ntex::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        std::env::set_var("RMQTT-CONFIG-FILENAME", args[1].clone());
    }
    logger_init();
    inner_api_serve_and_listen();
    plugin::init().await.expect("Failed to initialize plug-in");
    plugin::default_startups()
        .await
        .expect("Failed to startups plug-in");

    //hook, before startup
    Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .before_startup()
        .await;

    //tcp
    let mut tcp_listens = Vec::new();
    for (_, listen_cfg) in Runtime::instance().settings.listeners.tcps.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        tcp_listens.push(listen(name, listen_cfg));
    }

    //tls
    let mut tls_listens = Vec::new();
    for (_, listen_cfg) in Runtime::instance().settings.listeners.tlss.iter() {
        let name = format!("{}/{:?}", &listen_cfg.name, &listen_cfg.addr);
        tls_listens.push(listen_tls(name, listen_cfg));
    }

    let _ = futures::future::join(
        futures::future::join_all(tcp_listens),
        futures::future::join_all(tls_listens),
    )
    .await;
}

async fn listen(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen(name: &str, listen_cfg: &Listener) -> Result<()> {
        let max_inflight = listen_cfg.max_inflight;
        let handshake_timeout = listen_cfg.handshake_timeout.as_millis() as u16;
        let max_size = listen_cfg.max_packet_size.as_u32();
        let max_qos = listen_cfg.max_qos_allowed;
        let max_awaiting_rel = listen_cfg.max_awaiting_rel;
        let await_rel_timeout = listen_cfg.await_rel_timeout;
        ntex::server::Server::build()
            .bind(name, listen_cfg.addr, move || {
                MqttServer::new()
                    .v3(
                        v3::MqttServer::new(move |mut handshake: HandshakeV3<TcpStream>| async {
                            let remote_addr = handshake.io().peer_addr()?;
                            let local_addr = handshake.io().local_addr()?;
                            let listen_cfg = Runtime::instance()
                                .settings
                                .listeners
                                .tcp(local_addr.port())
                                .ok_or_else(|| {
                                    anyhow::Error::msg(format!(
                                        "tcp listener config is not found, local addr is {:?}",
                                        local_addr
                                    ))
                                })?;
                            let client_id = handshake.packet().client_id.clone();
                            match handshake_v3(listen_cfg, handshake, remote_addr, local_addr).await
                            {
                                Err(e) => {
                                    log::error!(
                                        "{:?}/{:?}/{} Connection Refused, handshake, {:?}",
                                        local_addr,
                                        remote_addr,
                                        client_id,
                                        e
                                    );
                                    Err(e)
                                }
                                Ok(ack) => Ok(ack),
                            }
                        })
                        // .v3(v3::MqttServer::new(handshake_v3)
                        .inflight(max_inflight)
                        .handshake_timeout(handshake_timeout)
                        .max_size(max_size)
                        .max_awaiting_rel(max_awaiting_rel)
                        .await_rel_timeout(await_rel_timeout)
                        .publish(fn_factory_with_config(
                            |session: v3::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    publish_v3(session.clone(), req)
                                }))
                            },
                        ))
                        .control(fn_factory_with_config(
                            |session: v3::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    control_message_v3(session.clone(), req)
                                }))
                            },
                        )),
                    )
                    .v5(
                        v5::MqttServer::new(move |mut handshake: HandshakeV5<TcpStream>| async {
                            let peer_addr = handshake.io().peer_addr()?;
                            let local_addr = handshake.io().local_addr()?;
                            let listen_cfg = Runtime::instance()
                                .settings
                                .listeners
                                .tcp(local_addr.port())
                                .ok_or_else(|| {
                                    anyhow::Error::msg(format!(
                                        "tcp listener config is not found, local addr is {:?}",
                                        local_addr
                                    ))
                                })?;
                            handshake_v5(listen_cfg, handshake, peer_addr, local_addr).await
                        })
                        //v5::MqttServer::new(handshake_v5)
                        //.receive_max()
                        .handshake_timeout(handshake_timeout)
                        .max_size(max_size)
                        .max_qos(max_qos)
                        //.max_topic_alias(max_topic_alias),
                        .publish(fn_factory_with_config(
                            |session: v5::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    publish_v5(session.clone(), req)
                                }))
                            },
                        ))
                        .control(fn_factory_with_config(
                            |session: v5::Session<SessionState>| {
                                ok::<_, MqttError>(fn_service(move |req| {
                                    control_message_v5(session.clone(), req)
                                }))
                            },
                        )),
                    )
            })?
            .workers(listen_cfg.workers)
            .maxconn(listen_cfg.max_connections / listen_cfg.workers)
            .backlog(listen_cfg.backlog)
            .run()
            .await?;
        Ok(())
    }

    _listen(&format!("tcp: {}", name), listen_cfg)
        .await
        .map_err(|e| {
            log::error!("Listen {:?} failed on {}, {:?}", name, listen_cfg.addr, e);
            e
        })
}

async fn listen_tls(name: String, listen_cfg: &Listener) -> Result<()> {
    async fn _listen_tls(name: &str, listen_cfg: &Listener) -> Result<()> {
        let mut tls_config = ServerConfig::new(NoClientAuth::new());

        let cert_file = &mut BufReader::new(File::open(listen_cfg.cert.as_ref().unwrap())?);
        let key_file = &mut BufReader::new(File::open(listen_cfg.key.as_ref().unwrap())?);

        let cert_chain = certs(cert_file).unwrap();
        let mut keys = rsa_private_keys(key_file).unwrap();
        tls_config.set_single_cert(cert_chain, keys.remove(0))?;

        let tls_acceptor = Acceptor::new(tls_config);

        let max_inflight = listen_cfg.max_inflight;
        let handshake_timeout = listen_cfg.handshake_timeout.as_millis() as u16;
        let max_size = listen_cfg.max_packet_size.as_u32();
        let max_qos = listen_cfg.max_qos_allowed;
        let max_awaiting_rel = listen_cfg.max_awaiting_rel;
        let await_rel_timeout = listen_cfg.await_rel_timeout;
        ntex::server::Server::build()
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
                                        .ok_or_else(|| anyhow::Error::msg(format!(
                                            "tls listener config is not found, local addr is {:?}",
                                            local_addr
                                        )))?;

                                    handshake_v3(listen_cfg, handshake, peer_addr, local_addr).await
                                },
                            )
                            //.v3(v3::MqttServer::new(handshake_v3)
                            .inflight(max_inflight)
                            .handshake_timeout(handshake_timeout)
                            .max_size(max_size)
                            .max_awaiting_rel(max_awaiting_rel)
                            .await_rel_timeout(await_rel_timeout)
                            .publish(fn_factory_with_config(
                                |session: v3::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        publish_v3(session.clone(), req)
                                    }))
                                },
                            ))
                            .control(fn_factory_with_config(
                                |session: v3::Session<SessionState>| {
                                    ok::<_, MqttError>(fn_service(move |req| {
                                        control_message_v3(session.clone(), req)
                                    }))
                                },
                            ))
                            )
                            .v5(
                                    //v5::MqttServer::new(handshake_v5)
                                    v5::MqttServer::new(move |mut handshake: HandshakeV5<TlsStream<TcpStream>>| async {
                                        let (io, _) = handshake.io().get_ref();
                                        let peer_addr = io.peer_addr()?;
                                        let local_addr = io.local_addr()?;
                                        let listen_cfg = Runtime::instance()
                                            .settings
                                            .listeners
                                            .tcp(local_addr.port())
                                            .ok_or_else(|| anyhow::Error::msg(format!(
                                                "tls listener config is not found, local addr is {:?}",
                                                local_addr
                                            )))?;
                                        handshake_v5(listen_cfg, handshake, peer_addr, local_addr).await
                                    })
                                    //.receive_max()
                                    .handshake_timeout(handshake_timeout)
                                    .max_size(max_size)
                                    .max_qos(max_qos)
                                    //.max_topic_alias(max_topic_alias)
                                    .publish(fn_factory_with_config(
                                        |session: v5::Session<SessionState>| {
                                            ok::<_, MqttError>(fn_service(move |req| {
                                                publish_v5(session.clone(), req)
                                            }))
                                        },
                                    ))
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
            .backlog(listen_cfg.backlog)
            .run()
            .await?;
        Ok(())
    }

    _listen_tls(&format!("tls: {}", name), listen_cfg)
        .await
        .map_err(|e| {
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

fn inner_api_serve_and_listen() {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .worker_threads(1)
            .thread_name("inner-api-worker")
            .thread_stack_size(4 * 1024 * 1024)
            .build()
            .unwrap();

        let inner_api = async {
            if let Err(e) = inner_api::serve("0.0.0.0:6767").await {
                log::error!("error: inner api listen on {}, {:?}", "0.0.0.0:6767", e);
            }
        };
        rt.block_on(inner_api);
    });
}
