use anyhow::Result;
use std::num::NonZeroU32;
use std::time::Duration;

use crate::broker::types::*;
use crate::settings::listener::Listener;

#[async_trait]
pub trait FitterManager: Sync + Send {
    fn get(&self, id: Id, listen_cfg: Listener) -> Box<dyn Fitter>;
}

#[async_trait]
pub trait Fitter: Sync + Send {
    ///keep_alive - is client input value, unit: seconds
    fn keep_alive(&self, keep_alive: u16) -> Result<u16>;

    ///Maximum length of message queue, default value: 1000
    fn max_mqueue_len(&self) -> usize;

    ///Pop up message speed from message queue, the number of pop-up messages from the queue within
    /// a specified time, which can effectively control the message flow rate,
    /// default value: 100 / 10s
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration);
}
