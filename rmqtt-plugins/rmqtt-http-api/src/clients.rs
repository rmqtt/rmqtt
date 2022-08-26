use rmqtt::{broker::Entry, ClientId, ClientInfo, Id, Runtime, Session, TimestampMillis};
use rmqtt::{chrono, futures};

use super::types::{ClientSearchParams as SearchParams, ClientSearchResult as SearchResult};

pub(crate) async fn get(clientid: &str) -> Option<SearchResult> {
    let shared = Runtime::instance().extends.shared().await;
    if !shared.exist(clientid) {
        return None;
    }

    let id = Id::from(Runtime::instance().node.id(), ClientId::from(clientid));
    let peer = shared.entry(id);
    let (s, c) = if let (Some(s), Some(c)) = (peer.session(), peer.client()) {
        (s, c)
    } else {
        return None;
    };
    Some(build_result(Some(s), Some(c)).await)
}

pub(crate) async fn search(q: &SearchParams) -> Vec<SearchResult> {
    let limit = q._limit;
    let mut curr: usize = 0;
    let peers = Runtime::instance()
        .extends
        .shared()
        .await
        .iter()
        .filter(|entry| filtering(q, entry.as_ref()))
        .filter_map(|entry| {
            if curr < limit {
                curr += 1;
                Some((entry.session(), entry.client()))
            } else {
                None
            }
        });

    let futs = peers.into_iter().map(|(s, c)| build_result(s, c)).collect::<Vec<_>>();
    futures::future::join_all(futs).await
}

async fn build_result(s: Option<Session>, c: Option<ClientInfo>) -> SearchResult {
    let s = if let Some(s) = s {
        s
    } else {
        return SearchResult::default();
    };
    let c = if let Some(c) = c {
        c
    } else {
        return SearchResult::default();
    };

    let connected = c.is_connected();
    let connected_at = c.connected_at / 1000;
    let disconnected_at = c.disconnected_at() / 1000;
    let disconnected_reason = c.get_disconnected_reason().await.unwrap_or_default();
    let expiry_interval = if connected {
        s.listen_cfg.session_expiry_interval.as_secs() as i64
    } else {
        s.listen_cfg.session_expiry_interval.as_secs() as i64
            - (chrono::Local::now().timestamp() - disconnected_at)
    };
    let inflight = s.inflight_win.read().await.len();
    SearchResult {
        node_id: c.id.node_id,
        clientid: c.id.client_id.clone(),
        username: c.username().clone(),
        proto_ver: c.connect_info.proto_ver(),
        ip_address: c.id.remote_addr.map(|addr| addr.ip().to_string()),
        port: c.id.remote_addr.map(|addr| addr.port()),
        connected,
        connected_at,
        disconnected_at,
        disconnected_reason,
        keepalive: c.connect_info.keep_alive(),
        clean_start: c.connect_info.clean_start(),
        session_present: c.session_present,
        expiry_interval,
        created_at: s.created_at / 1000,
        subscriptions_cnt: s.subscriptions().len(),
        max_subscriptions: s.listen_cfg.max_subscriptions,

        inflight,
        max_inflight: s.listen_cfg.max_inflight,

        mqueue_len: s.deliver_queue.len(),
        max_mqueue: s.listen_cfg.max_mqueue_len,
    }
}

fn filtering(q: &SearchParams, entry: &dyn Entry) -> bool {
    let s = if let Some(s) = entry.session() {
        s
    } else {
        return false;
    };

    let c = if let Some(c) = entry.client() {
        c
    } else {
        return false;
    };

    if let Some(clientid) = &q.clientid {
        if clientid.as_bytes() != s.id.client_id.as_bytes() {
            return false;
        }
    }

    if let Some(username) = &q.username {
        if username.as_bytes() != c.username().as_bytes() {
            return false;
        }
    }

    if let Some(ip_address) = &q.ip_address {
        if let Some(remote_addr) = c.id.remote_addr {
            if remote_addr.ip().to_string() != *ip_address {
                return false;
            }
        } else {
            return false;
        }
    }

    if let Some(connected) = &q.connected {
        if *connected != c.is_connected() {
            return false;
        }
    }

    if let Some(session_present) = &q.session_present {
        if *session_present != c.session_present {
            return false;
        }
    }

    if let Some(clean_start) = &q.clean_start {
        if *clean_start != c.connect_info.clean_start() {
            return false;
        }
    }

    if let Some(proto_ver) = &q.proto_ver {
        if *proto_ver != c.connect_info.proto_ver() {
            return false;
        }
    }

    if let Some(_like_clientid) = &q._like_clientid {
        if !c.id.client_id.contains(_like_clientid) {
            return false;
        }
    }

    if let Some(_like_username) = &q._like_username {
        if !c.username().contains(_like_username) {
            return false;
        }
    }

    if let Some(_gte_created_at) = &q._gte_created_at {
        if s.created_at < _gte_created_at.as_millis() as TimestampMillis {
            return false;
        }
    }

    if let Some(_lte_created_at) = &q._lte_created_at {
        if s.created_at > _lte_created_at.as_millis() as TimestampMillis {
            return false;
        }
    }

    if let Some(_gte_connected_at) = &q._gte_connected_at {
        if c.connected_at < _gte_connected_at.as_millis() as TimestampMillis {
            return false;
        }
    }

    if let Some(_lte_connected_at) = &q._lte_connected_at {
        if c.connected_at > _lte_connected_at.as_millis() as TimestampMillis {
            return false;
        }
    }

    if let Some(_gte_mqueue_len) = &q._gte_mqueue_len {
        if s.deliver_queue.len() < *_gte_mqueue_len {
            return false;
        }
    }

    if let Some(_lte_mqueue_len) = &q._lte_mqueue_len {
        if s.deliver_queue.len() > *_lte_mqueue_len {
            return false;
        }
    }

    true
}
