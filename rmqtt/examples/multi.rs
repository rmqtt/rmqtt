use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

#[tokio::main]
async fn main() -> Result<()> {
    std::env::set_var("RUST_LOG", "multi=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info");
    env_logger::init();

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .listener(Builder::new().name("internal/tcp").laddr(([0, 0, 0, 0], 11883).into()).bind()?.tcp()?)
        .listener(
            Builder::new()
                .name("external/tls")
                .laddr(([0, 0, 0, 0], 8883).into())
                .tls_key(Some("./rmqtt-bin/rmqtt.key"))
                .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
                .bind()?
                .tls()?,
        )
        .listener(Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?)
        .listener(
            Builder::new()
                .name("external/wss")
                .laddr(([0, 0, 0, 0], 8443).into())
                .tls_key(Some("./rmqtt-bin/rmqtt.key"))
                .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
                .bind()?
                .wss()?,
        )
        .build()
        .run()
        .await?;
    Ok(())
}
