use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

//cargo build -r --example plugin

#[tokio::main]
async fn main() -> Result<()> {
    std::env::set_var(
        "RUST_LOG",
        "plugin=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info,rmqtt_retainer=info",
    );
    env_logger::init();

    let scx = ServerContext::new().plugins_dir("rmqtt-plugins/").build().await;

    rmqtt_acl::register(&scx, "rmqtt-acl", true, false).await?;
    rmqtt_retainer::register(&scx, "rmqtt-retainer", true, false).await?;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
