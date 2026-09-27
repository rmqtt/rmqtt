//! Connection flapping protection for RMQTT.
//!
//! Counts connection attempts per ClientId, username and source IP address in a
//! sliding window, and refuses the offender with a ban once it crosses the
//! threshold — before authentication, so a broken device retrying in a loop, a
//! runaway client, or a credential-stuffing attempt costs the broker nothing
//! but a hash lookup.
//!
//! # Why this is a plugin
//!
//! The decision hangs on the `ClientConnect` hook, which every connection
//! reaches — including the ones that carry no username and therefore never
//! enter the authentication chain. Nothing in the broker core needs to change
//! to screen connections.
//!
//! The state, though, has to be readable from outside the plugin: an operator
//! wants to see who is banned and to lift a ban without restarting the broker.
//! `rmqtt-http-api` cannot let a plugin register a route, so the plugin
//! publishes a read view through the extension slot the core defines for
//! exactly this purpose (`rmqtt::flapping::Flapping`) — the same pattern
//! `retain`, `msgstore`, `delayed` and `auto-subscription` use.
//!
//! # Cluster semantics
//!
//! Both the counters and the ban table are node-local. Nothing ties a ClientId
//! to a node, so a client that reconnects through a load balancer spreads its
//! attempts over the cluster and is counted separately on each node. This
//! matches EMQX, and it means the threshold has to be set with the cluster size
//! in mind: with N nodes behind the balancer, an offender can make roughly N
//! times the attempts before any single node refuses it.
//!
//! # Layout
//!
//! * `config` — the configuration file.
//! * `gate` — the screening machinery: the `gate::Gate` trait, the ordered
//!   gate set, and the deadline index that keeps the periodic cleanup cheap.
//! * `gate::flapping` — the detector itself.
//! * `notify` — the `$SYS` announcements.
//! * `api_impl` — the read view the management API and the stats snapshot use.
//!
//! See `designs/flapping-plugin.md` for the design and its trade-offs.
#![deny(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::{spawn, sync::RwLock, task::JoinHandle};

use rmqtt::{
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Priority, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    utils::timestamp_millis,
    Result,
};

/// The current wall-clock time, in milliseconds since the UNIX epoch.
///
/// Every deadline in this plugin is an absolute millisecond stamp, so the
/// conversion from the broker's signed clock happens once, here. Mixing the two
/// signs is a mistake the compiler catches, not the reader.
#[inline]
pub(crate) fn now_ms() -> u64 {
    u64::try_from(timestamp_millis()).unwrap_or(0)
}

mod api_impl;
mod config;
mod gate;
mod notify;

use api_impl::FlappingApi;
use config::PluginConfig;
use gate::flapping::{Exempt, FlappingGate};
use gate::GateSet;
use notify::Notifier;

register!(FlappingPlugin::new);

#[derive(Plugin)]
struct FlappingPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    /// Kept apart from `gates` so that the read view can reach the tables
    /// without going through the trait object.
    gate: Arc<FlappingGate>,
    /// The gates screened on every connection, in order.
    gates: Arc<GateSet>,
    running: Arc<AtomicBool>,
    gc_handle: Option<JoinHandle<()>>,
}

/// Priority of the `ClientConnect` gate handler.
///
/// Deliberately below [`Priority::MAX`], where the plugins that merely observe
/// connections sit (`rmqtt-counter`, `rmqtt-web-hook`): those have to see every
/// CONNECT, refused ones included, because a refusal returns `proceed = false`
/// and drops the rest of the chain. Handlers sharing a priority are dispatched
/// in unspecified order (`rmqtt::hook::Priority`), so "after them" has to be
/// spelled as a smaller number — the order handlers were registered in plays no
/// part.
///
/// The midpoint leaves the whole upper half to observers and to any later gate
/// that must run earlier, while staying far above the default `0` of ordinary
/// handlers.
const CONNECT_GATE_PRIORITY: Priority = Priority::MAX / 2;

impl FlappingPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let mut cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;
        cfg.sanitize();
        log::info!(
            "{name} FlappingPlugin cfg: enable: {}, dimensions: {:?}, max_track: {}, gc_interval: {:?}",
            cfg.enable,
            cfg.enabled_dimensions(),
            cfg.max_track,
            cfg.gc_interval,
        );

        let cfg = Arc::new(RwLock::new(cfg));
        let notifier = Arc::new(Notifier::new(&scx));
        let gate = Arc::new(FlappingGate::new(&scx, cfg.clone(), notifier).await);
        let gates = Arc::new(GateSet::new(vec![Box::new(gate.clone())]));
        let register = scx.extends.hook_mgr().register();
        Ok(Self {
            scx,
            register,
            cfg,
            gate,
            gates,
            running: Arc::new(AtomicBool::new(false)),
            gc_handle: None,
        })
    }

    /// The sweep task.
    ///
    /// The interval is re-read every turn so that a reloaded configuration is
    /// honoured without restarting the task; `gc_once` itself only looks at the
    /// heads of the indexes, so an idle broker pays one clock reading per turn.
    async fn sweep(gates: Arc<GateSet>, cfg: Arc<RwLock<PluginConfig>>, running: Arc<AtomicBool>) {
        loop {
            let interval = cfg.read().await.gc_interval;
            tokio::time::sleep(interval).await;
            if running.load(Ordering::SeqCst) {
                gates.gc_once(now_ms());
            }
        }
    }
}

#[async_trait]
impl Plugin for FlappingPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());

        // `proceed = false` on a refusal drops the rest of the chain, so where
        // this handler sits matters: see `CONNECT_GATE_PRIORITY` for why the
        // gate runs below the observers instead of at the top.
        self.register
            .add_priority(
                Type::ClientConnect,
                CONNECT_GATE_PRIORITY,
                Box::new(FlappingHandler::new(self.gates.clone())),
            )
            .await;

        // Install the read view before the first connection can reach a ban, so
        // that the management API never reports "not loaded" for a plugin that
        // is loaded.
        *self.scx.extends.flapping_mut().await = Box::new(FlappingApi::new(self.gate.clone()));
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let mut new_cfg = self.scx.plugins.read_config_default::<PluginConfig>(self.name())?;
        new_cfg.sanitize();
        let enabled = new_cfg.is_any_dimension_enabled();
        // The exemption lists are read from a snapshot, so they are handed over
        // explicitly; the tables themselves are deliberately left alone, as
        // rebuilding them would drop every ban in force.
        self.gate.set_exempt(Exempt::from_config(&new_cfg));
        *self.cfg.write().await = new_cfg;
        if self.running.load(Ordering::SeqCst) {
            self.gate.set_enabled(enabled);
        }
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        self.gate.set_enabled(self.cfg.read().await.is_any_dimension_enabled());
        self.running.store(true, Ordering::SeqCst);
        self.gc_handle = Some(spawn(Self::sweep(self.gates.clone(), self.cfg.clone(), self.running.clone())));
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        self.running.store(false, Ordering::SeqCst);
        self.gate.set_enabled(false);
        if let Some(handle) = self.gc_handle.take() {
            handle.abort();
        }
        Ok(false)
    }
}

/// The hook adapter. It holds only the gates: everything else the decision
/// needs lives inside them.
struct FlappingHandler {
    gates: Arc<GateSet>,
}

impl FlappingHandler {
    #[inline]
    fn new(gates: Arc<GateSet>) -> Self {
        Self { gates }
    }
}

#[async_trait]
impl Handler for FlappingHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        if let Parameter::ClientConnect(ci) = param {
            if let Some(refuse) = self.gates.check(ci, now_ms()).await {
                // `proceed = false` with the refusal in the same result: the
                // accumulated result of a chain is a single slot, so a handler
                // that refuses must both stop the chain and supply the verdict.
                return (false, Some(HookResult::ConnectRefuse(refuse)));
            }
        }
        (true, acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate has to stay strictly below the observers' `Priority::MAX`.
    ///
    /// Handlers sharing a priority are dispatched in an order fixed by a
    /// randomly generated handler id, so a gate registered at `MAX` would win
    /// the coin flip about half of the broker processes and short-circuit
    /// `rmqtt-counter` / `rmqtt-web-hook` for every refused CONNECT in that
    /// process. The end-to-end case that counts those events
    /// (`rmqtt-test`, `flapping_observer_order_v5`) can therefore only catch
    /// such a regression half the time.
    ///
    /// The check sits in a const block, so a gate back at `Priority::MAX` fails
    /// the build instead of a test — and `clippy::assertions_on_constants`,
    /// which would fire on a plain `assert!`, stays quiet.
    #[test]
    fn connect_gate_priority_leaves_room_for_observers() {
        const { assert!(CONNECT_GATE_PRIORITY < Priority::MAX) };
    }
}
