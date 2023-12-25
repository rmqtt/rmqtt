use rmqtt::{
    broker::Entry, log, tokio, ClientId, ConnectInfo, Id, Result, Runtime, Session, TimestampMillis,
};
use rmqtt::{chrono, futures, serde_json};
use std::sync::Arc;

use super::types::{ClientSearchParams as SearchParams, ClientSearchResult as SearchResult};

pub(crate) async fn get(clientid: &str) -> Option<SearchResult> {
    let shared = Runtime::instance().extends.shared().await;
    if !shared.exist(clientid) {
        return None;
    }
    let id = Id::from(Runtime::instance().node.id(), ClientId::from(clientid));
    let peer = shared.entry(id);
    let s = peer.session()?;
    Some(build_result(Some(s)).await)
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
                Some(entry.session())
            } else {
                None
            }
        });

    let futs = peers.into_iter().map(build_result).collect::<Vec<_>>();
    futures::future::join_all(futs).await
}

async fn build_result(s: Option<Session>) -> SearchResult {
    let s = if let Some(s) = s {
        s
    } else {
        return SearchResult::default();
    };

    let connected = s.connected().await.unwrap_or_default();
    let connected_at = s.connected_at().await.map(|at| at / 1000).unwrap_or_default();
    let disconnected_at = s.disconnected_at().await.map(|at| at / 1000).unwrap_or_default();
    let disconnected_reason = s.disconnected_reason().await.map(|r| r.to_string()).unwrap_or_default();
    let d = s.disconnect().await.unwrap_or_default();
    let expiry_interval = if connected {
        s.fitter.session_expiry_interval(d.as_ref()).as_secs() as i64
    } else {
        s.fitter.session_expiry_interval(d.as_ref()).as_secs() as i64
            - (chrono::Local::now().timestamp() - disconnected_at)
    };
    let inflight = s.inflight_win().read().await.len();
    let created_at = s.created_at().await.map(|at| at / 1000).unwrap_or_default();
    let subscriptions_cnt = if let Ok(subs) = s.subscriptions().await { subs.len().await } else { 0 };
    let extra_attrs = s.extra_attrs.read().await.len();

    let connect_info = s.connect_info().await.ok();
    let last_will = connect_info
        .as_ref()
        .and_then(|conn_info| conn_info.last_will().map(|lw| lw.to_json()))
        .unwrap_or(serde_json::Value::Null);
    let keepalive = connect_info.as_ref().map(|c| c.keep_alive()).unwrap_or_default();
    let clean_start = connect_info.as_ref().map(|c| c.clean_start()).unwrap_or_default();
    let protocol = connect_info.as_ref().map(|c| c.proto_ver()).unwrap_or_default();
    let id = s.id.clone();
    SearchResult {
        node_id: id.node_id,
        clientid: id.client_id.clone(),
        username: id.username(),
        superuser: s.superuser().await.unwrap_or_default(),
        proto_ver: protocol,
        ip_address: id.remote_addr.map(|addr| addr.ip().to_string()),
        port: id.remote_addr.map(|addr| addr.port()),
        connected,
        connected_at,
        disconnected_at,
        disconnected_reason,
        keepalive,
        clean_start,
        session_present: s.session_present().await.unwrap_or_default(),
        expiry_interval,
        created_at,
        subscriptions_cnt,
        max_subscriptions: s.listen_cfg().max_subscriptions,
        extra_attrs,
        last_will,

        inflight,
        max_inflight: s.listen_cfg().max_inflight.get(),

        mqueue_len: s.deliver_queue().len(),
        max_mqueue: s.listen_cfg().max_mqueue_len,
    }
}

fn filtering(q: &SearchParams, entry: &dyn Entry) -> bool {
    tokio::task::block_in_place(move || {
        tokio::runtime::Handle::current().block_on(async move {
            match _filtering(q, entry).await {
                Ok(res) => res,
                Err(e) => {
                    log::error!("{:?}", e);
                    false
                }
            }
        })
    })
}

async fn _filtering(q: &SearchParams, entry: &dyn Entry) -> Result<bool> {
    let s = if let Some(s) = entry.session() {
        s
    } else {
        return Ok(false);
    };
    let id = &s.id;
    if let Some(clientid) = &q.clientid {
        if clientid.as_bytes() != id.client_id.as_bytes() {
            return Ok(false);
        }
    }

    if let Some(username) = &q.username {
        if username.as_bytes() != id.username_ref().as_bytes() {
            return Ok(false);
        }
    }

    if let Some(ip_address) = &q.ip_address {
        if let Some(remote_addr) = id.remote_addr {
            if remote_addr.ip().to_string() != *ip_address {
                return Ok(false);
            }
        } else {
            return Ok(false);
        }
    }

    if let Some(connected) = &q.connected {
        if *connected != s.connected().await.unwrap_or_default() {
            return Ok(false);
        }
    }

    if let Some(session_present) = &q.session_present {
        if *session_present != s.session_present().await.unwrap_or_default() {
            return Ok(false);
        }
    }

    let connect_info = s.connect_info().await.unwrap_or_else(|_| Arc::new(ConnectInfo::from(id.clone())));

    if let Some(clean_start) = &q.clean_start {
        if *clean_start != connect_info.clean_start() {
            return Ok(false);
        }
    }

    if let Some(proto_ver) = &q.proto_ver {
        if *proto_ver != connect_info.proto_ver() {
            return Ok(false);
        }
    }

    if let Some(_like_clientid) = &q._like_clientid {
        if !id.client_id.contains(_like_clientid) {
            return Ok(false);
        }
    }

    if let Some(_like_username) = &q._like_username {
        if !id.username_ref().contains(_like_username) {
            return Ok(false);
        }
    }

    let created_at = s.created_at().await.unwrap_or_default();

    if let Some(_gte_created_at) = &q._gte_created_at {
        if created_at < _gte_created_at.as_millis() as TimestampMillis {
            return Ok(false);
        }
    }

    if let Some(_lte_created_at) = &q._lte_created_at {
        if created_at > _lte_created_at.as_millis() as TimestampMillis {
            return Ok(false);
        }
    }

    let connected_at = s.connected_at().await.unwrap_or_default();

    if let Some(_gte_connected_at) = &q._gte_connected_at {
        if connected_at < _gte_connected_at.as_millis() as TimestampMillis {
            return Ok(false);
        }
    }

    if let Some(_lte_connected_at) = &q._lte_connected_at {
        if connected_at > _lte_connected_at.as_millis() as TimestampMillis {
            return Ok(false);
        }
    }

    if let Some(_gte_mqueue_len) = &q._gte_mqueue_len {
        if s.deliver_queue().len() < *_gte_mqueue_len {
            return Ok(false);
        }
    }

    if let Some(_lte_mqueue_len) = &q._lte_mqueue_len {
        if s.deliver_queue().len() > *_lte_mqueue_len {
            return Ok(false);
        }
    }

    Ok(true)
}
