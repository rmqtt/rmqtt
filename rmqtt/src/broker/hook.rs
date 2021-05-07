use crate::broker::types::*;
use crate::{Connection, Password, Session};

pub type Exit = bool;
pub type Failure = bool;
pub type BadUsernameOrPassword = bool;
pub type NotAuthorized = bool;
pub type Disconnect = bool;
pub type Deny = bool; //Reject current operation
pub type MessageExpiry = bool;

pub type Results = Vec<HookResult>;
pub type ReturnType = (bool, Results);

#[async_trait]
pub trait HookManager: Sync + Send {
    fn hook(&self, s: &Session, c: &Connection) -> std::rc::Rc<dyn Hook>;

    fn register(&self) -> Box<dyn Register>;

    ///Before the server startup
    async fn before_startup(&self) -> Exit;
}

pub trait Register: Sync + Send {
    fn add(&self, typ: Type, handler: Box<dyn Handler>);

    fn suspend(&self) {}

    fn resume(&self) {}
}

#[async_trait]
pub trait Handler: Sync + Send {
    async fn hook(&mut self, param: &Parameter, results: Results) -> ReturnType;
}

#[async_trait]
pub trait Hook: Sync + Send {
    ///If true is returned, the connection will be disconnected
    async fn session_created(&self) -> Disconnect;

    ///When sending mqtt:: connectack message
    async fn client_connack(&self, return_code: ConnectAckReason);

    ///If true is returned, the connection will be disconnected
    async fn client_connect(&self) -> Disconnect;

    ///authenticate
    async fn client_authenticate(&self, password: Option<Password>) -> ConnectAckReason;

    ///After the mqtt:: connectack message is sent, the connection is created successfully
    async fn client_connected(&self);

    ///Disconnect message received
    async fn client_disconnected(&self, r: Reason);

    ///Session terminated
    async fn session_terminated(&self, r: Reason);

    ///Subscribe message received
    ///client_subscribe_check_acl: is implemented based this hook
    async fn client_subscribe(&self, subscribe: &Subscribe) -> (Disconnect, Option<SubscribeAck>);

    ///Subscription succeeded
    async fn session_subscribed(&self, subscribed: Subscribed);

    ///Unsubscribe message received
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Disconnect;

    ///Unsubscribe succeeded
    async fn session_unsubscribed(&self, unsubscribed: Unsubscribed);

    ///Publish message received
    /// client_publish_check_acl: is implemented based this hook
    async fn message_publish(&self, p: &Publish) -> (Disconnect, Deny);

    ///Publish message Dropped
    async fn message_dropped(&self, to: Option<To>, from: From, p: Publish, reason: Reason);

    ///
    async fn message_deliver(&self, from: From, publish: &Publish);

    ///
    async fn message_acked(&self, from: From, publish: &Publish);

    ///
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiry;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Type {
    BeforeStartup,

    SessionCreated,
    SessionTerminated,
    SessionSubscribed,
    SessionUnsubscribed,

    ClientAuthenticate,
    ClientConnect,
    ClientConnack,
    ClientConnected,
    ClientDisconnected,
    ClientSubscribe,
    ClientUnsubscribe,

    MessagePublish,
    MessageDeliver,
    MessageAcked,
    MessageDropped,
    MessageExpiryCheck,
}

#[derive(Debug, Clone)]
pub enum Parameter<'a> {
    BeforeStartup,

    ClientAuthenticate(&'a Session, &'a Connection, Option<Password>),
    ClientConnect(&'a Session, &'a Connection),
    ClientConnack(&'a Session, &'a Connection, ConnectAckReason),
    ClientConnected(&'a Session, &'a Connection),
    ClientDisconnected(&'a Session, &'a Connection, Reason),
    ClientSubscribe(&'a Session, &'a Connection, &'a Subscribe),
    ClientUnsubscribe(&'a Session, &'a Connection, &'a Unsubscribe),

    SessionCreated(&'a Session, &'a Connection),
    SessionTerminated(&'a Session, &'a Connection, Reason),
    SessionSubscribed(&'a Session, &'a Connection, Subscribed),
    SessionUnsubscribed(&'a Session, &'a Connection, Unsubscribed),

    MessagePublish(&'a Session, &'a Connection, &'a Publish),
    MessageDeliver(&'a Session, &'a Connection, From, &'a Publish),
    MessageAcked(&'a Session, &'a Connection, From, &'a Publish),
    MessageDropped(
        &'a Session,
        &'a Connection,
        Option<To>,
        From,
        Publish,
        Reason,
    ),
    MessageExpiryCheck(&'a Session, &'a Connection, From, &'a Publish),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HookResult {
    ///Exit the program
    Exit,
    ///Disconnect
    Disconnect,
    ///Indicates that the current operation failed，Does not exit the program or close the connection
    Failure,
    ///Authentication failed
    BadUsernameOrPassword,
    ///Authentication failed
    NotAuthorized,
    ///CheckACL(Publish)
    Deny,
    // ///CheckACL(Subscribe), QoS - Is the currently allowed quality
    // Allow(QoS),
    ///Message Expiry
    MessageExpiry,
    ///Subscribe Ack
    SubscribeAck(SubscribeAck),
}
