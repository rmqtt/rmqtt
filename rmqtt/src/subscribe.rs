use async_trait::async_trait;

use crate::context::ServerContext;
use crate::types::*;
use crate::Result;

#[async_trait]
pub trait SharedSubscription: Sync + Send {
    ///Whether shared subscriptions are supported
    #[inline]
    fn is_supported(&self, listen_cfg: &ListenerConfig) -> bool {
        listen_cfg.shared_subscription
    }

    ///Shared subscription strategy, select a subscriber, default is "random"
    #[inline]
    async fn choice(
        &self,
        scx: &ServerContext,
        ncs: &[(
            NodeId,
            ClientId,
            SubscriptionOptions,
            Option<Vec<SubscriptionIdentifier>>,
            Option<IsOnline>,
        )],
    ) -> Option<(usize, IsOnline)> {
        if ncs.is_empty() {
            return None;
        }

        let mut tmp_ncs = ncs
            .iter()
            .enumerate()
            .map(|(idx, (node_id, client_id, _, _, is_online))| (idx, node_id, client_id, is_online))
            .collect::<Vec<_>>();

        while !tmp_ncs.is_empty() {
            let r_idx = if tmp_ncs.len() == 1 { 0 } else { (rand::random::<u64>() as usize) % tmp_ncs.len() };

            let (idx, node_id, client_id, is_online) = tmp_ncs.remove(r_idx);

            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };

            if is_online {
                return Some((idx, true));
            }

            if tmp_ncs.is_empty() {
                return Some((idx, is_online));
            }
        }
        return None;
    }
}

pub struct DefaultSharedSubscription;

#[async_trait]
impl SharedSubscription for DefaultSharedSubscription {}

#[async_trait]
pub trait AutoSubscription: Sync + Send {
    #[inline]
    fn enable(&self) -> bool {
        false
    }

    #[inline]
    async fn subscribe(&self, _id: &Id, _msg_tx: &Tx) -> Result<()> {
        Ok(())
    }
}

pub struct DefaultAutoSubscription;

#[async_trait]
impl AutoSubscription for DefaultAutoSubscription {}
