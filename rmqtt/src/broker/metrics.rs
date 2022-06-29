use std::sync::atomic::{AtomicUsize, Ordering};
use once_cell::sync::OnceCell;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Metrics {

    client_authenticate: AtomicUsize,
    client_auth_anonymous: AtomicUsize,
    client_connect: AtomicUsize,
    client_connack: AtomicUsize,
    client_connected: AtomicUsize,
    client_disconnected: AtomicUsize,
    client_subscribe_check_acl: AtomicUsize,
    client_publish_check_acl: AtomicUsize,

    publishs: AtomicUsize,
    delivers: AtomicUsize,
    ackeds: AtomicUsize,
}

impl Clone for Metrics{
    fn clone(&self) -> Self{
        Self{
            client_authenticate: AtomicUsize::new(self.client_authenticate.load(Ordering::SeqCst)),
            client_auth_anonymous: AtomicUsize::new(self.client_auth_anonymous.load(Ordering::SeqCst)),
            client_connect: AtomicUsize::new(self.client_connect.load(Ordering::SeqCst)),
            client_connack: AtomicUsize::new(self.client_connack.load(Ordering::SeqCst)),
            client_connected: AtomicUsize::new(self.client_connected.load(Ordering::SeqCst)),
            client_disconnected: AtomicUsize::new(self.client_disconnected.load(Ordering::SeqCst)),
            client_subscribe_check_acl: AtomicUsize::new(self.client_subscribe_check_acl.load(Ordering::SeqCst)),
            client_publish_check_acl: AtomicUsize::new(self.client_publish_check_acl.load(Ordering::SeqCst)),

            publishs: AtomicUsize::new(self.publishs.load(Ordering::SeqCst)),
            delivers: AtomicUsize::new(self.delivers.load(Ordering::SeqCst)),
            ackeds: AtomicUsize::new(self.ackeds.load(Ordering::SeqCst)),
        }
    }
}

impl Metrics {
    #[inline]
    pub fn instance() -> &'static Metrics {
        static INSTANCE: OnceCell<Metrics> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            client_authenticate: AtomicUsize::new(0),
            client_auth_anonymous: AtomicUsize::new(0),
            client_connect: AtomicUsize::new(0),
            client_connack: AtomicUsize::new(0),
            client_connected: AtomicUsize::new(0),
            client_disconnected: AtomicUsize::new(0),
            client_subscribe_check_acl: AtomicUsize::new(0),
            client_publish_check_acl: AtomicUsize::new(0),

            publishs: AtomicUsize::new(0),
            delivers: AtomicUsize::new(0),
            ackeds: AtomicUsize::new(0),
        })
    }


    #[inline]
    pub fn client_authenticate_inc(&self) {
        self.client_authenticate.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_auth_anonymous_inc(&self) {
        self.client_auth_anonymous.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_connect_inc(&self) {
        self.client_connect.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_connack_inc(&self) {
        self.client_connack.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_connected_inc(&self) {
        self.client_connected.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_disconnected_inc(&self) {
        self.client_disconnected.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_subscribe_check_acl_inc(&self) {
        self.client_subscribe_check_acl.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn client_publish_check_acl_inc(&self) {
        self.client_publish_check_acl.fetch_add(1, Ordering::SeqCst);
    }


    #[inline]
    pub fn publishs_inc(&self) {
        self.publishs.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn delivers_inc(&self) {
        self.delivers.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn ackeds_inc(&self) {
        self.ackeds.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value{
        json!({
            "client.authenticate": self.client_authenticate.load(Ordering::SeqCst),
            "client.auth_anonymous": self.client_auth_anonymous.load(Ordering::SeqCst),
            "client.connect": self.client_connect.load(Ordering::SeqCst),
            "client.connack": self.client_connack.load(Ordering::SeqCst),
            "client.connected": self.client_connected.load(Ordering::SeqCst),
            "client.disconnected": self.client_disconnected.load(Ordering::SeqCst),
            "client.subscribe_check_acl": self.client_subscribe_check_acl.load(Ordering::SeqCst),
            "client.publish_check_acl": self.client_publish_check_acl.load(Ordering::SeqCst),

            "publishs": self.publishs.load(Ordering::SeqCst),
            "delivers": self.delivers.load(Ordering::SeqCst),
            "ackeds": self.ackeds.load(Ordering::SeqCst),
        })
    }

}