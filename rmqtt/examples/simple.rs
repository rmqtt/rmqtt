use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

#[tokio::main]
async fn main() -> Result<()> {
    std::env::set_var("RUST_LOG", "simple=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info");
    env_logger::init();

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
