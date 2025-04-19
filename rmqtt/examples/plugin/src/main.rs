use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let scx = ServerContext::new().plugins_dir("rmqtt-plugins/").build().await;

    rmqtt_acl::register(&scx, "rmqtt-acl", true, false).await?;
    // rmqtt_auth_http::register(&scx, "rmqtt-auth-http", true, false).await?;
    rmqtt_retainer::register(&scx, "rmqtt-retainer", true, false).await?;
    rmqtt_http_api::register(&scx, "rmqtt-http-api", true, false).await?;
    rmqtt_web_hook::register(&scx, "rmqtt-web-hook", true, false).await?;
    // rmqtt_auth_jwt::register(&scx, "rmqtt-auth-jwt", true, false).await?;
    // rmqtt_counter::register(&scx, "rmqtt-counter", true, false).await?;
    // rmqtt_bridge_egress_kafka::register(&scx, "rmqtt-bridge-egress-kafka", true, false).await?;
    // rmqtt_bridge_ingress_kafka::register(&scx, "rmqtt-bridge-ingress-kafka", true, false).await?;
    // rmqtt_auto_subscription::register(&scx, "rmqtt-auto-subscription", true, false).await?;
    // rmqtt_message_storage::register(&scx, "rmqtt-message-storage", true, false).await?;
    // rmqtt_session_storage::register(&scx, "rmqtt-session-storage", true, false).await?;
    // rmqtt_sys_topic::register(&scx, "rmqtt-sys-topic", true, false).await?;
    // rmqtt_topic_rewrite::register(&scx, "rmqtt-topic-rewrite", true, false).await?;
    // rmqtt_bridge_egress_mqtt::register(&scx, "rmqtt-bridge-egress-mqtt", true, false).await?;
    // rmqtt_bridge_ingress_mqtt::register(&scx, "rmqtt-bridge-ingress-mqtt", true, false).await?;
    // rmqtt_bridge_egress_pulsar::register(&scx, "rmqtt-bridge-egress-pulsar", true, false).await?;
    // rmqtt_bridge_ingress_pulsar::register(&scx, "rmqtt-bridge-ingress-pulsar", true, false).await?;
    // rmqtt_bridge_egress_nats::register(&scx, "rmqtt-bridge-egress-nats", true, false).await?;
    // rmqtt_bridge_egress_reductstore::register(&scx, "rmqtt-bridge-egress-reductstore", true, false).await?;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
