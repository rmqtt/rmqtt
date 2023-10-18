use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::Result;

pub trait FitterManager: Sync + Send {
    fn create(&self, conn_info: Arc<ConnectInfo>, id: Id, listen_cfg: Listener) -> FitterType;
}

#[async_trait]
pub trait Fitter: Sync + Send {
    ///keep_alive - is client input value, unit: seconds
    fn keep_alive(&self, keep_alive: &mut u16) -> Result<u16>;

    ///Maximum length of message queue, default value: 1000
    fn max_mqueue_len(&self) -> usize;

    ///Pop up message speed from message queue, the number of pop-up messages from the queue within
    /// a specified time, which can effectively control the message flow rate,
    /// default value: 100 / 10s
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration);

    ///max inflight
    fn max_inflight(&self) -> std::num::NonZeroU16;

    ///session expiry interval
    async fn session_expiry_interval(&self, d: Option<&Disconnect>) -> Duration;

    ///client topic alias maximum, C -> S(Max Limit)
    fn max_client_topic_aliases(&self) -> u16;

    ///server topic alias maximum, S(Max Limit) -> C
    fn max_server_topic_aliases(&self) -> u16;
}
