use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

//cargo build -r --example simple_wss  --features tls,ws

#[tokio::main]
async fn main() -> Result<()> {
    // std::env::set_var("RUST_LOG", "simple_wss=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info");
    // env_logger::init();

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
