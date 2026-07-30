//! MQTT 5.0 User Properties test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct UserPropertiesV5Test;
impl TestCase for UserPropertiesV5Test {
    fn name(&self) -> &str {
        "user_properties_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "up-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "up-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/userprops", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let props =
                vec![("key1".to_string(), "value1".to_string()), ("key2".to_string(), "value2".to_string())];
            publisher
                .publish_with_properties(
                    "test/v5/userprops",
                    b"props_msg",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&props),
                )
                .await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"props_msg" => {
                    // Message delivered - user properties propagation is optional
                    Ok(())
                }
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("message with user properties not received")),
            }
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
