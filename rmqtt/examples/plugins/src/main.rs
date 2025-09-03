use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let scx = ServerContext::new().plugins_config_dir("rmqtt-plugins/").build().await;

    rmqtt_plugins::acl::register(&scx, true, false).await?;
    rmqtt_plugins::http_api::register(&scx, true, false).await?;
    rmqtt_plugins::retainer::register(&scx, true, false).await?;
    // rmqtt_plugins::sys_topic::register(&scx, true, false).await?;
    // rmqtt_plugins::message_storage::register(&scx, true, false).await?;

    // rmqtt_plugins::session_storage::register(&scx, true, false).await?;
    // rmqtt_plugins::auth_jwt::register(&scx, true, false).await?;
    // rmqtt_plugins::auth_http::register(&scx, true, false).await?;
    // rmqtt_plugins::web_hook::register(&scx, true, false).await?;
    // rmqtt_plugins::counter::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_egress_kafka::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_ingress_kafka::register(&scx, true, false).await?;
    // rmqtt_plugins::auto_subscription::register(&scx, true, false).await?;
    // rmqtt_plugins::topic_rewrite::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_egress_mqtt::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_ingress_mqtt::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_egress_pulsar::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_ingress_pulsar::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_egress_nats::register(&scx, true, false).await?;
    // rmqtt_plugins::bridge_egress_reductstore::register(&scx, true, false).await?;
    //
    // rmqtt_plugins::cluster_raft::register(&scx, true, true).await?;
    // rmqtt_plugins::cluster_broadcast::register(&scx, true, true).await?;

    MqttServer::new(scx)
        .listener(
            Builder::new()
                .name("external/tcp")
                .laddr(([0, 0, 0, 0], 1883).into())
                .allow_anonymous(false)
                .bind()?
                .tcp()?,
        )
        .build()
        .run()
        .await?;
    Ok(())
}
