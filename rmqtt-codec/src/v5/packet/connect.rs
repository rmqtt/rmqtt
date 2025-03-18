use std::num::{NonZeroU16, NonZeroU32};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use crate::error::{DecodeError, EncodeError};
use crate::types::{ConnectFlags, QoS, MQTT, MQTT_LEVEL_5, WILL_QOS_SHIFT};
use crate::utils::{self, Decode, Encode, Property};
use crate::v5::{encode::*, property_type as pt, UserProperties, UserProperty};

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
/// Connect packet content
pub struct Connect {
    /// the handling of the Session state.
    pub clean_start: bool,
    /// a time interval measured in seconds.
    pub keep_alive: u16,

    pub session_expiry_interval_secs: u32,
    pub auth_method: Option<ByteString>,
    pub auth_data: Option<Bytes>,
    pub request_problem_info: bool,
    pub request_response_info: bool,
    pub receive_max: Option<NonZeroU16>,
    pub topic_alias_max: u16,
    pub user_properties: UserProperties,
    pub max_packet_size: Option<NonZeroU32>,

    /// Will Message be stored on the Server and associated with the Network Connection.
    pub last_will: Option<LastWill>,
    /// identifies the Client to the Server.
    pub client_id: ByteString,
    /// username can be used by the Server for authentication and authorization.
    pub username: Option<ByteString>,
    /// password can be used by the Server for authentication and authorization.
    pub password: Option<Bytes>,
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
/// Connection Will
pub struct LastWill {
    /// the QoS level to be used when publishing the Will Message.
    pub qos: QoS,
    /// the Will Message is to be Retained when it is published.
    pub retain: bool,
    /// the Will Topic
    pub topic: ByteString,
    /// defines the Application Message that is to be published to the Will Topic
    pub message: Bytes,

    pub will_delay_interval_sec: Option<u32>,
    pub correlation_data: Option<Bytes>,
    pub message_expiry_interval: Option<NonZeroU32>,
    pub content_type: Option<ByteString>,
    pub user_properties: UserProperties,
    pub is_utf8_payload: Option<bool>,
    pub response_topic: Option<ByteString>,
}

impl LastWill {
    fn properties_len(&self) -> usize {
        encoded_property_size(&self.will_delay_interval_sec)
            + encoded_property_size(&self.correlation_data)
            + encoded_property_size(&self.message_expiry_interval)
            + encoded_property_size(&self.content_type)
            + encoded_property_size(&self.is_utf8_payload)
            + encoded_property_size(&self.response_topic)
            + self.user_properties.encoded_size()
    }
}

impl Connect {
    /// Set client_id value
    pub fn client_id<T>(mut self, client_id: T) -> Self
    where
        ByteString: From<T>,
    {
        self.client_id = client_id.into();
        self
    }

    /// Set receive_max value
    pub fn receive_max(mut self, max: u16) -> Self {
        if let Some(num) = NonZeroU16::new(max) {
            self.receive_max = Some(num);
        } else {
            self.receive_max = None;
        }
        self
    }

    fn properties_len(&self) -> usize {
        encoded_property_size(&self.auth_method)
            + encoded_property_size(&self.auth_data)
            + encoded_property_size_default(&self.session_expiry_interval_secs, 0)
            + encoded_property_size_default(&self.request_problem_info, true) // 3.1.2.11.7 Request Problem Information
            + encoded_property_size_default(&self.request_response_info, false) // 3.1.2.11.6 Request Response Information
            + encoded_property_size(&self.receive_max)
            + encoded_property_size(&self.max_packet_size)
            + encoded_property_size_default(&self.topic_alias_max, 0)
            + self.user_properties.encoded_size()
    }

    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.remaining() >= 10, DecodeError::InvalidLength);
        let len = src.get_u16();

        ensure!(len == 4 && &src.as_ref()[0..4] == MQTT, DecodeError::InvalidProtocol);
        src.advance(4);

        let level = src.get_u8();
        ensure!(level == MQTT_LEVEL_5, DecodeError::UnsupportedProtocolLevel);

        let flags = ConnectFlags::from_bits(src.get_u8()).ok_or(DecodeError::ConnectReservedFlagSet)?;
        let keep_alive = src.get_u16();

        // reading properties
        let mut session_expiry_interval_secs = None;
        let mut auth_method = None;
        let mut auth_data = None;
        let mut request_problem_info = None;
        let mut request_response_info = None;
        let mut receive_max = None;
        let mut topic_alias_max = None;
        let mut user_properties = Vec::new();
        let mut max_packet_size = None;
        let prop_src = &mut utils::take_properties(src)?;
        while prop_src.has_remaining() {
            match prop_src.get_u8() {
                pt::SESS_EXPIRY_INT => session_expiry_interval_secs.read_value(prop_src)?,
                pt::AUTH_METHOD => auth_method.read_value(prop_src)?,
                pt::AUTH_DATA => auth_data.read_value(prop_src)?,
                pt::REQ_PROB_INFO => request_problem_info.read_value(prop_src)?,
                pt::REQ_RESP_INFO => request_response_info.read_value(prop_src)?,
                pt::RECEIVE_MAX => receive_max.read_value(prop_src)?,
                pt::TOPIC_ALIAS_MAX => topic_alias_max.read_value(prop_src)?,
                pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
                pt::MAX_PACKET_SIZE => max_packet_size.read_value(prop_src)?,
                _ => return Err(DecodeError::MalformedPacket),
            }
        }

        let client_id = ByteString::decode(src)?;

        ensure!(
            // todo: [MQTT-3.1.3-8]?
            !client_id.is_empty() || flags.contains(ConnectFlags::CLEAN_START),
            DecodeError::InvalidClientId
        );

        let last_will =
            if flags.contains(ConnectFlags::WILL) { Some(decode_last_will(src, flags)?) } else { None };

        let username =
            if flags.contains(ConnectFlags::USERNAME) { Some(ByteString::decode(src)?) } else { None };
        let password = if flags.contains(ConnectFlags::PASSWORD) { Some(Bytes::decode(src)?) } else { None };

        Ok(Connect {
            clean_start: flags.contains(ConnectFlags::CLEAN_START),
            keep_alive,
            session_expiry_interval_secs: session_expiry_interval_secs.unwrap_or(0),
            auth_method,
            auth_data,
            receive_max,
            topic_alias_max: topic_alias_max.unwrap_or(0u16),
            request_problem_info: request_problem_info.unwrap_or(true),
            request_response_info: request_response_info.unwrap_or(false),
            user_properties,
            max_packet_size,

            client_id,
            last_will,
            username,
            password,
        })
    }
}

impl Default for Connect {
    fn default() -> Connect {
        Connect {
            clean_start: false,
            keep_alive: 0,
            session_expiry_interval_secs: 0,
            auth_method: None,
            auth_data: None,
            request_problem_info: true,
            request_response_info: false,
            receive_max: None,
            topic_alias_max: 0,
            user_properties: Vec::new(),
            max_packet_size: None,
            last_will: None,
            client_id: ByteString::default(),
            username: None,
            password: None,
        }
    }
}

fn decode_last_will(src: &mut Bytes, flags: ConnectFlags) -> Result<LastWill, DecodeError> {
    let mut will_delay_interval_sec = None;
    let mut correlation_data = None;
    let mut message_expiry_interval = None;
    let mut content_type = None;
    let mut user_properties = Vec::new();
    let mut is_utf8_payload = None;
    let mut response_topic = None;
    let prop_src = &mut utils::take_properties(src)?;
    while prop_src.has_remaining() {
        match prop_src.get_u8() {
            pt::WILL_DELAY_INT => will_delay_interval_sec.read_value(prop_src)?,
            pt::CORR_DATA => correlation_data.read_value(prop_src)?,
            pt::MSG_EXPIRY_INT => message_expiry_interval.read_value(prop_src)?,
            pt::CONTENT_TYPE => content_type.read_value(prop_src)?,
            pt::UTF8_PAYLOAD => is_utf8_payload.read_value(prop_src)?,
            pt::RESP_TOPIC => response_topic.read_value(prop_src)?,
            pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
            _ => return Err(DecodeError::MalformedPacket),
        }
    }

    let topic = ByteString::decode(src)?;
    let message = Bytes::decode(src)?;
    Ok(LastWill {
        qos: QoS::try_from((flags & ConnectFlags::WILL_QOS).bits() >> WILL_QOS_SHIFT)?,
        retain: flags.contains(ConnectFlags::WILL_RETAIN),
        topic,
        message,
        will_delay_interval_sec,
        correlation_data,
        message_expiry_interval,
        content_type,
        user_properties,
        is_utf8_payload,
        response_topic,
    })
}

impl EncodeLtd for Connect {
    fn encoded_size(&self, _limit: u32) -> usize {
        let prop_len = self.properties_len();
        6 // protocol name
            + 1 // protocol level
            + 1 // connect flags
            + 2 // keep alive
            + var_int_len(prop_len) as usize // properties len
            + prop_len // properties
            + self.client_id.encoded_size()
            + self.last_will.as_ref().map_or(0, |will| { // will message content
                let prop_len = will.properties_len();
                var_int_len(prop_len) as usize + prop_len + will.topic.encoded_size() + will.message.encoded_size()
            })
            + self.username.as_ref().map_or(0, |v| v.encoded_size())
            + self.password.as_ref().map_or(0, |v| v.encoded_size())
    }

    fn encode(&self, buf: &mut BytesMut, _size: u32) -> Result<(), EncodeError> {
        b"MQTT".as_ref().encode(buf)?;

        let mut flags = ConnectFlags::empty();

        if self.username.is_some() {
            flags |= ConnectFlags::USERNAME;
        }
        if self.password.is_some() {
            flags |= ConnectFlags::PASSWORD;
        }

        if let Some(will) = self.last_will.as_ref() {
            flags |= ConnectFlags::WILL;

            if will.retain {
                flags |= ConnectFlags::WILL_RETAIN;
            }

            flags |= ConnectFlags::from_bits_truncate(u8::from(will.qos) << WILL_QOS_SHIFT);
        }

        if self.clean_start {
            flags |= ConnectFlags::CLEAN_START;
        }

        buf.put_slice(&[MQTT_LEVEL_5, flags.bits()]);

        self.keep_alive.encode(buf)?;

        let prop_len = self.properties_len();
        utils::write_variable_length(prop_len as u32, buf); // safe: whole message size is vetted via max size check in codec

        encode_property_default(&self.session_expiry_interval_secs, 0, pt::SESS_EXPIRY_INT, buf)?;
        encode_property(&self.auth_method, pt::AUTH_METHOD, buf)?;
        encode_property(&self.auth_data, pt::AUTH_DATA, buf)?;
        encode_property_default(&self.request_problem_info, true, pt::REQ_PROB_INFO, buf)?; // 3.1.2.11.7 Request Problem Information
        encode_property_default(&self.request_response_info, false, pt::REQ_RESP_INFO, buf)?; // 3.1.2.11.6 Request Response Information
        encode_property(&self.receive_max, pt::RECEIVE_MAX, buf)?;
        encode_property(&self.max_packet_size, pt::MAX_PACKET_SIZE, buf)?;
        encode_property_default(&self.topic_alias_max, 0, pt::TOPIC_ALIAS_MAX, buf)?;
        self.user_properties.encode(buf)?;

        self.client_id.encode(buf)?;

        if let Some(will) = self.last_will.as_ref() {
            let prop_len = will.properties_len();
            utils::write_variable_length(prop_len as u32, buf); // safe: whole message size is checked for max already

            encode_property(&will.will_delay_interval_sec, pt::WILL_DELAY_INT, buf)?;
            encode_property(&will.is_utf8_payload, pt::UTF8_PAYLOAD, buf)?;
            encode_property(&will.message_expiry_interval, pt::MSG_EXPIRY_INT, buf)?;
            encode_property(&will.content_type, pt::CONTENT_TYPE, buf)?;
            encode_property(&will.response_topic, pt::RESP_TOPIC, buf)?;
            encode_property(&will.correlation_data, pt::CORR_DATA, buf)?;
            will.user_properties.encode(buf)?;
            will.topic.encode(buf)?;
            will.message.encode(buf)?;
        }
        if let Some(s) = self.username.as_ref() {
            s.encode(buf)?;
        }
        if let Some(pwd) = self.password.as_ref() {
            pwd.encode(buf)?;
        }
        Ok(())
    }
}
