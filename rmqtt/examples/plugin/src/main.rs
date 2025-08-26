use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    std::env::set_var("RUST_LOG", "info");
    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            use std::io::Write;
            let file = record
                .file()
                .and_then(|f| Path::new(f).file_name().and_then(|n| n.to_str()))
                .unwrap_or("unknown");
            writeln!(
                buf,
                "[{} {} ({}:{})] {}",
                record.level(),
                record.module_path().unwrap_or("unknown"),
                file,
                record.line().unwrap_or(0),
                record.args()
            )
        })
        .init();

    let rules = r###"rules = [
                ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],
                ["allow", { ipaddr = "127.0.0.1" }, "pubsub", ["$SYS/#", "#"]],
                ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
                ["allow", "all"]
        ]"###;

    let scx = ServerContext::new()
        .plugins_config_dir("rmqtt-plugins/")
        .plugins_config_map_add("rmqtt-acl", rules)
        .build()
        .await;

    rmqtt_acl::register(&scx, true, false).await?;
    rmqtt_http_api::register(&scx, true, false).await?;
    // rmqtt_message_storage::register(&scx, true, false).await?;
    rmqtt_retainer::register(&scx, true, false).await?;
    rmqtt_sys_topic::register(&scx, true, false).await?;
    // rmqtt_session_storage::register(&scx, true, false).await?;
    // rmqtt_auth_jwt::register(&scx, true, false).await?;
    // rmqtt_auth_http::register(&scx, true, false).await?;
    // rmqtt_web_hook::register(&scx, true, false).await?;
    // rmqtt_counter::register(&scx, true, false).await?;
    // rmqtt_bridge_egress_kafka::register(&scx, true, false).await?;
    // rmqtt_bridge_ingress_kafka::register(&scx, true, false).await?;
    // rmqtt_auto_subscription::register(&scx, true, false).await?;
    // rmqtt_topic_rewrite::register(&scx, true, false).await?;
    // rmqtt_bridge_egress_mqtt::register(&scx, true, false).await?;
    // rmqtt_bridge_ingress_mqtt::register(&scx, true, false).await?;
    // rmqtt_bridge_egress_pulsar::register(&scx, true, false).await?;
    // rmqtt_bridge_ingress_pulsar::register(&scx, true, false).await?;
    // rmqtt_bridge_egress_nats::register(&scx, true, false).await?;
    // rmqtt_bridge_egress_reductstore::register(&scx, true, false).await?;

    // rmqtt_cluster_raft::register(&scx, true, true).await?;
    // rmqtt_cluster_broadcast::register(&scx, true, true).await?;

    MqttServer::new(scx)
        .listener(
            Builder::new()
                .name("external/tcp")
                .laddr(([0, 0, 0, 0], 1883).into())
                .allow_anonymous(false)
                // .max_inflight(std::num::NonZeroU16::new(1).unwrap())
                .bind()?
                .tcp()?,
        )
        .build()
        .run()
        .await?;
    Ok(())
}
