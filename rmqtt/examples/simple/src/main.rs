use log::LevelFilter;
use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(LevelFilter::Info).init()?;

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
