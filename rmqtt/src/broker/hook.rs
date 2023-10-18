use crate::broker::types::*;
use crate::{grpc, Result, Session};

pub type Priority = u32;
pub type Proceed = bool;
pub type ReturnType = (Proceed, Option<HookResult>);

#[async_trait]
pub trait HookManager: Sync + Send {
    fn hook(&self, s: &Session) -> std::rc::Rc<dyn Hook>;

    fn register(&self) -> Box<dyn Register>;

    ///Before the server startup
    async fn before_startup(&self);

    ///When a connect message is received
    async fn client_connect(&self, connect_info: &ConnectInfo) -> Option<UserProperties>;

    ///authenticate
    async fn client_authenticate(
        &self,
        connect_info: &ConnectInfo,
        allow_anonymous: bool,
    ) -> (ConnectAckReason, Superuser);

    ///When sending mqtt:: connectack message
    async fn client_connack(
        &self,
        connect_info: &ConnectInfo,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason;

    ///Publish message received
    async fn message_publish(&self, s: Option<&Session>, from: From, publish: &Publish) -> Option<Publish>;

    ///Publish message Dropped
    async fn message_dropped(&self, to: Option<To>, from: From, p: Publish, reason: Reason);

    ///Publish message nonsubscribed
    async fn message_nonsubscribed(&self, from: From);

    ///grpc message received
    async fn grpc_message_received(
        &self,
        typ: grpc::MessageType,
        msg: grpc::Message,
    ) -> Result<grpc::MessageReply>;
}

#[async_trait]
pub trait Register: Sync + Send {
    async fn add(&self, typ: Type, handler: Box<dyn Handler>) {
        self.add_priority(typ, 0, handler).await;
    }

    async fn add_priority(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>);

    async fn start(&self) {}

    async fn stop(&self) {}
}

#[async_trait]
pub trait Handler: Sync + Send {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType;
}

#[async_trait]
pub trait Hook: Sync + Send {
    ///session created
    async fn session_created(&self);

    ///After the mqtt:: connectack message is sent, the connection is created successfully
    async fn client_connected(&self);

    ///Disconnect message received
    async fn client_disconnected(&self, r: Reason);

    ///Session terminated
    async fn session_terminated(&self, r: Reason);

    ///subscribe check acl
    async fn client_subscribe_check_acl(&self, subscribe: &Subscribe) -> Option<SubscribeAclResult>;

    ///publish check acl
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishAclResult;

    ///Subscribe message received
    async fn client_subscribe(&self, subscribe: &Subscribe) -> Option<TopicFilter>;

    ///Subscription succeeded
    async fn session_subscribed(&self, subscribe: Subscribe);

    ///Unsubscribe message received
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Option<TopicFilter>;

    ///Unsubscribe succeeded
    async fn session_unsubscribed(&self, unsubscribe: Unsubscribe);

    ///Publish message received
    async fn message_publish(&self, from: From, p: &Publish) -> Option<Publish>;

    ///message delivered
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish>;

    ///message acked
    async fn message_acked(&self, from: From, publish: &Publish);

    ///message expiry check
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiryCheckResult;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Deserialize, Serialize)]
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
    ClientSubscribeCheckAcl,

    MessagePublishCheckAcl,
    MessagePublish,
    MessageDelivered,
    MessageAcked,
    MessageDropped,
    MessageExpiryCheck,
    MessageNonsubscribed,

    GrpcMessageReceived,
}

impl std::convert::From<&str> for Type {
    fn from(t: &str) -> Type {
        match t {
            "before_startup" => Type::BeforeStartup,

            "session_created" => Type::SessionCreated,
            "session_terminated" => Type::SessionTerminated,
            "session_subscribed" => Type::SessionSubscribed,
            "session_unsubscribed" => Type::SessionUnsubscribed,

            "client_authenticate" => Type::ClientAuthenticate,
            "client_connect" => Type::ClientConnect,
            "client_connack" => Type::ClientConnack,
            "client_connected" => Type::ClientConnected,
            "client_disconnected" => Type::ClientDisconnected,
            "client_subscribe" => Type::ClientSubscribe,
            "client_unsubscribe" => Type::ClientUnsubscribe,
            "client_subscribe_check_acl" => Type::ClientSubscribeCheckAcl,

            "message_publish_check_acl" => Type::MessagePublishCheckAcl,
            "message_publish" => Type::MessagePublish,
            "message_delivered" => Type::MessageDelivered,
            "message_acked" => Type::MessageAcked,
            "message_dropped" => Type::MessageDropped,
            "message_expiry_check" => Type::MessageExpiryCheck,
            "message_nonsubscribed" => Type::MessageNonsubscribed,

            "grpc_message_received" => Type::GrpcMessageReceived,

            _ => unreachable!("{:?} is not defined", t),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Parameter<'a> {
    BeforeStartup,

    SessionCreated(&'a Session),
    SessionTerminated(&'a Session, Reason),
    SessionSubscribed(&'a Session, Subscribe),
    SessionUnsubscribed(&'a Session, Unsubscribe),

    ClientConnect(&'a ConnectInfo),
    ClientConnack(&'a ConnectInfo, &'a ConnectAckReason),
    ClientAuthenticate(&'a ConnectInfo),
    ClientConnected(&'a Session),
    ClientDisconnected(&'a Session, Reason),
    ClientSubscribe(&'a Session, &'a Subscribe),
    ClientUnsubscribe(&'a Session, &'a Unsubscribe),
    ClientSubscribeCheckAcl(&'a Session, &'a Subscribe),

    MessagePublishCheckAcl(&'a Session, &'a Publish),
    MessagePublish(Option<&'a Session>, From, &'a Publish),
    MessageDelivered(&'a Session, From, &'a Publish),
    MessageAcked(&'a Session, From, &'a Publish),
    MessageDropped(Option<To>, From, Publish, Reason),
    MessageExpiryCheck(&'a Session, From, &'a Publish),
    MessageNonsubscribed(From),

    GrpcMessageReceived(grpc::MessageType, grpc::Message),
}

impl<'a> Parameter<'a> {
    pub fn get_type(&self) -> Type {
        match self {
            Parameter::BeforeStartup => Type::BeforeStartup,

            Parameter::SessionCreated(_) => Type::SessionCreated,
            Parameter::SessionTerminated(_, _) => Type::SessionTerminated,
            Parameter::SessionSubscribed(_, _) => Type::SessionSubscribed,
            Parameter::SessionUnsubscribed(_, _) => Type::SessionUnsubscribed,

            Parameter::ClientAuthenticate(_) => Type::ClientAuthenticate,
            Parameter::ClientConnect(_) => Type::ClientConnect,
            Parameter::ClientConnack(_, _) => Type::ClientConnack,
            Parameter::ClientConnected(_) => Type::ClientConnected,
            Parameter::ClientDisconnected(_, _) => Type::ClientDisconnected,
            Parameter::ClientSubscribe(_, _) => Type::ClientSubscribe,
            Parameter::ClientUnsubscribe(_, _) => Type::ClientUnsubscribe,
            Parameter::ClientSubscribeCheckAcl(_, _) => Type::ClientSubscribeCheckAcl,

            Parameter::MessagePublishCheckAcl(_, _) => Type::MessagePublishCheckAcl,
            Parameter::MessagePublish(_, _, _) => Type::MessagePublish,
            Parameter::MessageDelivered(_, _, _) => Type::MessageDelivered,
            Parameter::MessageAcked(_, _, _) => Type::MessageAcked,
            Parameter::MessageDropped(_, _, _, _) => Type::MessageDropped,
            Parameter::MessageExpiryCheck(_, _, _) => Type::MessageExpiryCheck,
            Parameter::MessageNonsubscribed(_) => Type::MessageNonsubscribed,

            Parameter::GrpcMessageReceived(_, _) => Type::GrpcMessageReceived,
        }
    }
}

#[derive(Debug)]
pub enum HookResult {
    ///User Properties, for ClientConnect
    UserProperties(UserProperties),
    ///Authentication failed, for ClientAuthenticate
    AuthResult(AuthResult),
    ///ConnectAckReason, for ClientConnack
    ConnectAckReason(ConnectAckReason),
    ///TopicFilters, for ClientSubscribe/ClientUnsubscribe
    TopicFilter(Option<TopicFilter>),
    ///Subscribe AclResult, for ClientSubscribeCheckAcl
    SubscribeAclResult(SubscribeAclResult),
    ///Publish AclResult, for MessagePublishCheckAcl
    PublishAclResult(PublishAclResult),
    ///Publish, for MessagePublish/MessageDelivered
    Publish(Publish),
    ///Message Expiry
    MessageExpiry,
    ///for GrpcMessageReceived
    GrpcMessageReply(Result<grpc::MessageReply>),
}
