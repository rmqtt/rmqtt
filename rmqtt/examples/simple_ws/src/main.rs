use log::LevelFilter;
use simple_logger::SimpleLogger;

use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(LevelFilter::Info).init()?;

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?)
        .build()
        .run()
        .await?;
    Ok(())
}
