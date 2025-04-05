use log::LevelFilter;
use simple_logger::SimpleLogger;

use rmqtt::context::ServerContext;
use rmqtt::net::{Builder, Result};
use rmqtt::server::MqttServer;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(LevelFilter::Info).init()?;

    let scx = ServerContext::new().plugins_dir("rmqtt-plugins/").build().await;

    rmqtt_acl::register(&scx, "rmqtt-acl", true, false).await?;
    rmqtt_auth_http::register(&scx, "rmqtt-auth-http", true, false).await?;
    rmqtt_auth_jwt::register(&scx, "rmqtt-auth-jwt", true, false).await?;
    rmqtt_http_api::register(&scx, "rmqtt-http-api", true, false).await?;
    rmqtt_counter::register(&scx, "rmqtt-counter", true, false).await?;
    rmqtt_retainer::register(&scx, "rmqtt-retainer", true, false).await?;
    rmqtt_bridge_egress_kafka::register(&scx, "rmqtt-bridge-egress-kafka", true, false).await?;
    rmqtt_auto_subscription::register(&scx, "rmqtt-auto-subscription", true, false).await?;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
