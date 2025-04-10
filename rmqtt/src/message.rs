use std::time::Duration;

use async_trait::async_trait;

use crate::types::{ClientId, From, MsgID, Publish, SharedGroup, TopicFilter};
use crate::Result;

#[async_trait]
pub trait MessageManager: Sync + Send {
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        0
    }

    ///Store messages
    ///
    ///_msg_id           - MsgID <br>
    ///_from             - From <br>
    ///_p                - Message <br>
    ///_expiry_interval  - Message expiration time <br>
    ///_sub_client_ids   - Indicate that certain subscribed clients have already forwarded the specified message.
    #[inline]
    async fn store(
        &self,
        _msg_id: MsgID,
        _from: From,
        _p: Publish,
        _expiry_interval: Duration,
        _sub_client_ids: Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>,
    ) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn get(
        &self,
        _client_id: &str,
        _topic_filter: &str,
        _group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        Ok(Vec::new())
    }

    ///Indicate whether merging data from various nodes is needed during the 'get' operation.
    #[inline]
    fn should_merge_on_get(&self) -> bool {
        false
    }

    #[inline]
    async fn count(&self) -> isize {
        -1
    }

    #[inline]
    async fn max(&self) -> isize {
        -1
    }

    #[inline]
    fn enable(&self) -> bool {
        false
    }
}

pub struct DefaultMessageManager {}

impl Default for DefaultMessageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultMessageManager {
    #[inline]
    pub fn new() -> DefaultMessageManager {
        Self {}
    }
}

impl MessageManager for DefaultMessageManager {}
