use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

//cargo build -r --example simple_tls  --features tls

#[tokio::main]
async fn main() -> Result<()> {
    // std::env::set_var("RUST_LOG", "simple_tls=debug,rmqtt=info,rmqtt_net=info,rmqtt_codec=info");
    // env_logger::init();

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(
            Builder::new()
                .name("external/tls")
                .laddr(([0, 0, 0, 0], 8883).into())
                .tls_key(Some("./rmqtt-bin/rmqtt.key"))
                .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
                .bind()?
                .tls()?,
        )
        .build()
        .run()
        .await?;
    Ok(())
}
