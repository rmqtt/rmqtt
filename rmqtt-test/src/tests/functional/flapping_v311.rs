//! Connection flapping gate — MQTT 3.1.1 (rmqtt-flapping)
//!
//! The 3.1.1 half of the gate `flapping_v5.rs` describes. Both cases run
//! against the same fixture (`configs/flapping/`, started by the harness via
//! `broker_config()` and reached at its own `broker_addr()`), which is what
//! makes the two versions comparable: the gate counts and bans identically,
//! and only the wire representation of the refusal differs.
//!
//! The point this case pins is the *fallback*. MQTT 3.1.1 defines six CONNACK
//! return codes and none of them means "banned", so `ConnectRefuse::Banned`
//! has to be expressed as the closest code the Client can act on:
//! `0x05 Not authorized` (`ConnectRefuse::to_v3`). A client that reconnects in
//! a loop must not be told 0x03 Service unavailable, which would invite an
//! immediate retry — the ban is a policy decision, not a transient failure.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::connack_return_codes_v311::{build_connect, connect_return_code};
use crate::tests::functional::flapping_v5::{
    await_sys_event, cross_threshold, flapping_config, run, sys_watcher, ACCEPTED, BAN_TIME, BROKER_ADDR,
    REASON_BANNED_V311, SUITE_V311, SYS_BAN_FILTER,
};

/// The ClientId this case spends its budget on, and its `$SYS` watcher.
///
/// Both are deliberately distinct from the v5 case's: the two cases run against
/// the same fixture, hence the same broker process — the same config and the
/// same `broker_addr()` mean the scheduler never restarts the broker between the
/// `@flapping` sub-suites — so a key shared across cases would hand one case's
/// leftover window to the next one's budget.
const VICTIM_BAN_V311: &str = "flapping-victim-ban-v311";
const WATCHER_BAN_V311: &str = "flapping-watcher-ban-v311";

/// The threshold refusal over MQTT 3.1.1, and the lapse of the ban.
pub struct FlappingBanV311Test;

impl TestCase for FlappingBanV311Test {
    fn name(&self) -> &str {
        "flapping_ban_v311"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        flapping_config()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        // Broker start + the attempts + BAN_TIME + headroom.
        Duration::from_secs(45)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(SUITE_V311, self.name(), async {
            let addr = BROKER_ADDR;
            // clean session = 1, no credentials: the fixture allows anonymous
            // access, so only the gate can refuse this CONNECT.
            let packet = build_connect(0x02, VICTIM_BAN_V311, None, None);
            let mut watcher = sys_watcher(&ctx.config, WATCHER_BAN_V311, SYS_BAN_FILTER).await?;

            let refused = cross_threshold(addr, &packet, VICTIM_BAN_V311)?;
            if refused != REASON_BANNED_V311 {
                return Err(anyhow!(
                    "{VICTIM_BAN_V311}: MQTT 3.1.1 has no `Banned` CONNACK code, so a ban must fall \
                     back to 0x{REASON_BANNED_V311:02x} (Not authorized); got 0x{refused:02x} \
                     (0x03 would tell the client to retry at once)"
                ));
            }

            // The refusal is announced on $SYS whatever the protocol version
            // the refused client spoke.
            await_sys_event(&mut watcher, "banned", VICTIM_BAN_V311, Duration::from_secs(10))
                .await
                .ok_or_else(|| {
                    anyhow!(
                        "no `banned` notice for {VICTIM_BAN_V311} arrived on {SYS_BAN_FILTER} within \
                         10s, although the CONNECT was refused with 0x{refused:02x}"
                    )
                })?;

            // The ban lapses on its own, and one attempt then suffices: the
            // window was discarded when the ban was created.
            tokio::time::sleep(BAN_TIME + Duration::from_millis(700)).await;
            let after = connect_return_code(addr, &packet)?
                .ok_or_else(|| anyhow!("{VICTIM_BAN_V311}: no CONNACK after the ban lapsed"))?;
            if after != ACCEPTED {
                return Err(anyhow!(
                    "{VICTIM_BAN_V311}: the ban is still in force {:?} after it was created, \
                     although ban_time = {BAN_TIME:?} — CONNACK 0x{after:02x}",
                    BAN_TIME + Duration::from_millis(700),
                ));
            }

            watcher.disconnect().await?;
            Ok(())
        })
    }
}
