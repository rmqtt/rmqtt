use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

//cargo build -r --example simple_ws --features ws

#[tokio::main]
async fn main() -> Result<()> {
    // std::env::set_var("RUST_LOG", "simple_ws=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info");
    // env_logger::init();

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?)
        .build()
        .run()
        .await?;
    Ok(())
}
