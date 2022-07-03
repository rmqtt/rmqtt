use once_cell::sync::OnceCell;

use crate::{broker::{metrics::Metrics, stats::Stats}, extend, node::Node, plugin, settings::Settings};
use crate::logger::{config_logger, Logger};

pub struct Runtime {
    pub logger: Logger,
    pub settings: Settings,
    pub extends: extend::Manager,
    pub plugins: plugin::Manager,
    pub node: Node,
    pub metrics: &'static Metrics,
    pub stats: &'static Stats,
}

impl Runtime {
    #[inline]
    pub fn instance() -> &'static Self {
        static INSTANCE: OnceCell<Runtime> = OnceCell::new();
        INSTANCE.get_or_init(|| {
            let settings = Settings::new().unwrap();
            Self {
                logger: config_logger(
                    settings.log.filename(),
                    settings.log.to.clone(),
                    settings.log.level.clone(),
                ),
                settings: settings.clone(),
                extends: extend::Manager::new(),
                plugins: plugin::Manager::new(),
                node: Node::new(),
                metrics: Metrics::instance(),
                stats: Stats::instance(),
            }
        })
    }
}
