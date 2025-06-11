use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
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
