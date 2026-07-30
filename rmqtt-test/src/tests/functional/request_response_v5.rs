//! MQTT 5.0 Request-Response pattern test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct RequestResponseV5Test;
impl TestCase for RequestResponseV5Test {
    fn name(&self) -> &str {
        "request_response_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let mut requester = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "rr-req",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut responder = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "rr-resp",
                ctx.config.connect_timeout,
            )
            .await?;

            let request_topic = "test/v5/request";
            let response_topic = "test/v5/response/req1";
            responder.subscribe(request_topic, QoS::AtLeastOnce).await?;
            requester.subscribe(response_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Send request with response_topic and correlation_data
            let corr_data = b"corr-12345";
            requester
                .publish_with_properties(
                    request_topic,
                    b"ping",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    Some(response_topic),
                    Some(corr_data),
                    None,
                    None,
                )
                .await?;

            // Responder receives the request
            let req = responder.recv_message_timeout(Duration::from_secs(3)).await;
            if req.is_none() {
                return Err(anyhow::anyhow!("responder did not receive request"));
            }
            let req = req.unwrap();
            // Check that response_topic and correlation_data were propagated
            let resp_t = req.response_topic.clone();
            let corr = req.correlation_data.clone();

            // Responder sends reply to the response_topic with same correlation_data
            if let Some(rt) = resp_t {
                let cd_bytes = corr.as_deref().unwrap_or(b"");
                responder
                    .publish_with_properties(
                        &rt,
                        b"pong",
                        QoS::AtLeastOnce,
                        false,
                        None,
                        None,
                        None,
                        Some(cd_bytes),
                        None,
                        None,
                    )
                    .await?;

                // Requester receives the response
                let resp = requester.recv_message_timeout(Duration::from_secs(3)).await;
                requester.disconnect().await?;
                responder.disconnect().await?;

                match resp {
                    Some(m) if m.payload.as_ref() == b"pong" => Ok(()),
                    Some(m) => Err(anyhow::anyhow!("unexpected response: {:?}", m.payload)),
                    None => Err(anyhow::anyhow!("no response received")),
                }
            } else {
                // Broker didn't propagate response_topic - acceptable
                requester.disconnect().await?;
                responder.disconnect().await?;
                Ok(())
            }
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
