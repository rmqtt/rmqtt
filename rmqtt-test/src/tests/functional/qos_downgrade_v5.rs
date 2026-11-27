//! MQTT 5.0 QoS downgrade matrix (G13)
//!
//! designs/mqtt-5.0-standalone-test-gap-analysis.md
//!
//! The QoS of a delivered message is the minimum of the publishing QoS and
//! the maximum QoS granted by the matching subscription.
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::{QoS, QoSTest};

/// G13: QoS downgrade matrix — for each (publish QoS, subscribe QoS) pair the
/// broker must deliver with min(publish QoS, granted subscription QoS).
pub struct QosDowngradeV5MatrixTest;

impl QosDowngradeV5MatrixTest {
    async fn run_case(ctx: &TestContext, pub_qos: QoSTest, sub_qos: QoSTest) -> anyhow::Result<()> {
        let topic = format!(
            "test/v5/downgrade/{}{}",
            match pub_qos {
                QoS::AtMostOnce => "q0",
                QoS::AtLeastOnce => "q1",
                QoS::ExactlyOnce => "q2",
            },
            match sub_qos {
                QoS::AtMostOnce => "s0",
                QoS::AtLeastOnce => "s1",
                QoS::ExactlyOnce => "s2",
            },
        );
        let payload = format!("downgrade-{}", topic);

        let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
            &ctx.config.broker_addr,
            "dg-sub",
            ctx.config.connect_timeout,
        )
        .await?;
        subscriber.subscribe(&topic, sub_qos).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;

        let publisher = crate::mqtt::v5::MqttV5Client::connect(
            &ctx.config.broker_addr,
            "dg-pub",
            ctx.config.connect_timeout,
        )
        .await?;
        publisher.publish(&topic, payload.as_bytes(), pub_qos, false).await?;

        // Give the broker time to deliver before tearing down.
        let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
        publisher.disconnect().await?;
        subscriber.disconnect().await?;

        let expected_qos = pub_qos.less_value(sub_qos);
        match msg {
            Some(m) if m.payload.as_ref() == payload.as_bytes() => {
                if m.qos == expected_qos {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "delivered QoS is not min(publish, subscription): got {:?}, expected {:?} \
                         (publish {:?}, subscribe {:?})",
                        m.qos,
                        expected_qos,
                        pub_qos,
                        sub_qos
                    ))
                }
            }
            Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
            None => {
                Err(anyhow::anyhow!("no message delivered (publish {:?}, subscribe {:?})", pub_qos, sub_qos))
            }
        }
    }
}

impl TestCase for QosDowngradeV5MatrixTest {
    fn name(&self) -> &str {
        "qos_downgrade_v5_matrix"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        // QoS 0 publishes cannot be downgraded further; cover the pairs where
        // the publish QoS exceeds the subscription QoS.
        let matrix = [
            (QoSTest::AtLeastOnce, QoSTest::AtMostOnce),  // 1 -> 0
            (QoSTest::ExactlyOnce, QoSTest::AtMostOnce),  // 2 -> 0
            (QoSTest::ExactlyOnce, QoSTest::AtLeastOnce), // 2 -> 1
        ];

        let mut failures = Vec::new();
        for (pub_qos, sub_qos) in matrix {
            if let Err(e) = rt.block_on(Self::run_case(ctx, pub_qos, sub_qos)) {
                failures.push(format!("{:?}->{:?}: {}", pub_qos, sub_qos, e));
            }
        }

        if failures.is_empty() {
            TestResult::passed(self.name(), "functional_v5", start.elapsed())
        } else {
            TestResult::failed(self.name(), "functional_v5", start.elapsed(), failures.join("; "))
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}
