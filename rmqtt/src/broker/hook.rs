use crate::broker::types::*;
use crate::{Connection, Password, Session};

pub type ReturnType = (bool, Option<HookResult>);

#[async_trait]
pub trait HookManager: Sync + Send {
    fn hook(&self, s: &Session, c: &Connection) -> std::rc::Rc<dyn Hook>;

    fn register(&self) -> Box<dyn Register>;

    ///Before the server startup
    async fn before_startup(&self);

    ///When a connect message is received
    async fn client_connect<'a>(&'a self, connect_info: Connect<'a>) -> Option<UserProperties>;

    ///When sending mqtt:: connectack message
    async fn client_connack<'a>(
        &'a self,
        connect_info: Connect<'a>,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason;
}

pub trait Register: Sync + Send {
    fn add(&self, typ: Type, handler: Box<dyn Handler>);

    fn start(&self) {}

    fn stop(&self) {}
}

#[async_trait]
pub trait Handler: Sync + Send {
    async fn hook(&mut self, param: &Parameter, acc: Option<HookResult>) -> ReturnType;
}

#[async_trait]
pub trait Hook: Sync + Send {
    ///session created
    async fn session_created(&self);

    ///authenticate
    async fn client_authenticate(&self, password: Option<Password>) -> ConnectAckReason;

    ///After the mqtt:: connectack message is sent, the connection is created successfully
    async fn client_connected(&self);

    ///Disconnect message received
    async fn client_disconnected(&self, r: Reason);

    ///Session terminated
    async fn session_terminated(&self, r: Reason);

    ///subscribe check acl
    async fn client_subscribe_check_acl(&self, subscribe: &Subscribe)
        -> Option<SubscribeACLResult>;

    ///publish check acl
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishACLResult;

    ///Subscribe message received
    async fn client_subscribe(&self, subscribe: &Subscribe) -> Option<TopicFilters>;

    ///Subscription succeeded
    async fn session_subscribed(&self, subscribed: Subscribed);

    ///Unsubscribe message received
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Option<TopicFilters>;

    ///Unsubscribe succeeded
    async fn session_unsubscribed(&self, unsubscribed: Unsubscribed);

    ///Publish message received
    async fn message_publish(&self, p: &Publish) -> Option<Publish>;

    ///Publish message Dropped
    async fn message_dropped(&self, to: Option<To>, from: From, p: Publish, reason: Reason);

    ///message delivered
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish>;

    ///message acked
    async fn message_acked(&self, from: From, publish: &Publish);

    ///message expiry check
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
    ClientSubscribeCheckACL,

    MessagePublishCheckACL,
    MessagePublish,
    MessageDelivered,
    MessageAcked,
    MessageDropped,
    MessageExpiryCheck,
}

#[derive(Debug, Clone)]
pub enum Parameter<'a> {
    BeforeStartup,

    SessionCreated(&'a Session, &'a Connection),
    SessionTerminated(&'a Session, &'a Connection, Reason),
    SessionSubscribed(&'a Session, &'a Connection, Subscribed),
    SessionUnsubscribed(&'a Session, &'a Connection, Unsubscribed),

    ClientConnect(Connect<'a>),
    ClientConnack(Connect<'a>, &'a ConnectAckReason),
    ClientAuthenticate(&'a Session, &'a Connection, Option<Password>),
    ClientConnected(&'a Session, &'a Connection),
    ClientDisconnected(&'a Session, &'a Connection, Reason),
    ClientSubscribe(&'a Session, &'a Connection, &'a Subscribe),
    ClientUnsubscribe(&'a Session, &'a Connection, &'a Unsubscribe),
    ClientSubscribeCheckACL(&'a Session, &'a Connection, &'a Subscribe),

    MessagePublishCheckACL(&'a Session, &'a Connection, &'a Publish),
    MessagePublish(&'a Session, &'a Connection, &'a Publish),
    MessageDelivered(&'a Session, &'a Connection, From, &'a Publish),
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

#[derive(Debug, Clone)]
pub enum HookResult {
    ///User Properties, for ClientConnect
    UserProperties(UserProperties),
    ///Authentication failed, for ClientAuthenticate
    AuthResult(AuthResult),
    ///ConnectAckReason, for ClientConnack
    ConnectAckReason(ConnectAckReason),
    ///TopicFilters, for ClientSubscribe/ClientUnsubscribe
    TopicFilters(TopicFilters),
    ///Subscribe ACLResult, for ClientSubscribeCheckACL
    SubscribeACLResult(SubscribeACLResult),
    ///Publish ACLResult, for MessagePublishCheckACL
    PublishACLResult(PublishACLResult),
    ///Publish, for MessagePublish/MessageDelivered
    Publish(Publish),
    ///Message Expiry
    MessageExpiry,
}
