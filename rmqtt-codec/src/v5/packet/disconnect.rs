use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use crate::error::{DecodeError, EncodeError};
use crate::utils::{self, Decode, Property};
use crate::v5::{encode::*, property_type as pt, UserProperties, UserProperty};

/// DISCONNECT message
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct Disconnect {
    pub reason_code: DisconnectReasonCode,
    pub session_expiry_interval_secs: Option<u32>,
    pub server_reference: Option<ByteString>,
    pub reason_string: Option<ByteString>,
    pub user_properties: UserProperties,
}

pub trait ToReasonCode {
    fn to_reason_code(&self) -> DisconnectReasonCode;
}

prim_enum! {
    /// DISCONNECT reason codes
    #[derive(Deserialize, Serialize)]
    pub enum DisconnectReasonCode {
        NormalDisconnection = 0,
        DisconnectWithWillMessage = 4,
        UnspecifiedError = 128,
        MalformedPacket = 129,
        ProtocolError = 130,
        ImplementationSpecificError = 131,
        NotAuthorized = 135,
        ServerBusy = 137,
        ServerShuttingDown = 139,
        BadAuthenticationMethod = 140,
        KeepAliveTimeout = 141,
        SessionTakenOver = 142,
        TopicFilterInvalid = 143,
        TopicNameInvalid = 144,
        ReceiveMaximumExceeded = 147,
        TopicAliasInvalid = 148,
        PacketTooLarge = 149,
        MessageRateTooHigh = 150,
        QuotaExceeded = 151,
        AdministrativeAction = 152,
        PayloadFormatInvalid = 153,
        RetainNotSupported = 154,
        QosNotSupported = 155,
        UseAnotherServer = 156,
        ServerMoved = 157,
        SharedSubscriptionNotSupported = 158,
        ConnectionRateExceeded = 159,
        MaximumConnectTime = 160,
        SubscriptionIdentifiersNotSupported = 161,
        WildcardSubscriptionsNotSupported = 162
    }
}

impl From<DisconnectReasonCode> for u8 {
    fn from(v: DisconnectReasonCode) -> Self {
        match v {
            DisconnectReasonCode::NormalDisconnection => 0,
            DisconnectReasonCode::DisconnectWithWillMessage => 4,
            DisconnectReasonCode::UnspecifiedError => 128,
            DisconnectReasonCode::MalformedPacket => 129,
            DisconnectReasonCode::ProtocolError => 130,
            DisconnectReasonCode::ImplementationSpecificError => 131,
            DisconnectReasonCode::NotAuthorized => 135,
            DisconnectReasonCode::ServerBusy => 137,
            DisconnectReasonCode::ServerShuttingDown => 139,
            DisconnectReasonCode::BadAuthenticationMethod => 140,
            DisconnectReasonCode::KeepAliveTimeout => 141,
            DisconnectReasonCode::SessionTakenOver => 142,
            DisconnectReasonCode::TopicFilterInvalid => 143,
            DisconnectReasonCode::TopicNameInvalid => 144,
            DisconnectReasonCode::ReceiveMaximumExceeded => 147,
            DisconnectReasonCode::TopicAliasInvalid => 148,
            DisconnectReasonCode::PacketTooLarge => 149,
            DisconnectReasonCode::MessageRateTooHigh => 150,
            DisconnectReasonCode::QuotaExceeded => 151,
            DisconnectReasonCode::AdministrativeAction => 152,
            DisconnectReasonCode::PayloadFormatInvalid => 153,
            DisconnectReasonCode::RetainNotSupported => 154,
            DisconnectReasonCode::QosNotSupported => 155,
            DisconnectReasonCode::UseAnotherServer => 156,
            DisconnectReasonCode::ServerMoved => 157,
            DisconnectReasonCode::SharedSubscriptionNotSupported => 158,
            DisconnectReasonCode::ConnectionRateExceeded => 159,
            DisconnectReasonCode::MaximumConnectTime => 160,
            DisconnectReasonCode::SubscriptionIdentifiersNotSupported => 161,
            DisconnectReasonCode::WildcardSubscriptionsNotSupported => 162,
        }
    }
}

impl Disconnect {
    /// Create new instance of `Disconnect` with specified code
    pub fn new(reason_code: DisconnectReasonCode) -> Self {
        Self {
            reason_code,
            session_expiry_interval_secs: None,
            server_reference: None,
            reason_string: None,
            user_properties: Vec::new(),
        }
    }

    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let disconnect = if src.has_remaining() {
            let reason_code = src.get_u8().try_into()?;

            if src.has_remaining() {
                let mut session_exp_secs = None;
                let mut server_reference = None;
                let mut reason_string = None;
                let mut user_properties = Vec::new();

                let prop_src = &mut utils::take_properties(src)?;
                while prop_src.has_remaining() {
                    match prop_src.get_u8() {
                        pt::SESS_EXPIRY_INT => session_exp_secs.read_value(prop_src)?,
                        pt::REASON_STRING => reason_string.read_value(prop_src)?,
                        pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
                        pt::SERVER_REF => server_reference.read_value(prop_src)?,
                        _ => return Err(DecodeError::MalformedPacket),
                    }
                }
                ensure!(!src.has_remaining(), DecodeError::InvalidLength);

                Self {
                    reason_code,
                    session_expiry_interval_secs: session_exp_secs,
                    server_reference,
                    reason_string,
                    user_properties,
                }
            } else {
                Self { reason_code, ..Default::default() }
            }
        } else {
            Self::default()
        };
        Ok(disconnect)
    }
}

impl Default for Disconnect {
    fn default() -> Self {
        Self {
            reason_code: DisconnectReasonCode::NormalDisconnection,
            session_expiry_interval_secs: None,
            server_reference: None,
            reason_string: None,
            user_properties: Vec::new(),
        }
    }
}

impl EncodeLtd for Disconnect {
    fn encoded_size(&self, limit: u32) -> usize {
        const HEADER_LEN: usize = 1; // reason code

        let mut prop_len = encoded_property_size(&self.session_expiry_interval_secs)
            + encoded_property_size(&self.server_reference);
        let diag_len = encoded_size_opt_props(
            &self.user_properties,
            &self.reason_string,
            reduce_limit(limit, prop_len + HEADER_LEN + 4),
        ); // exclude other props and max of 4 bytes for property length value
        prop_len += diag_len;
        HEADER_LEN + var_int_len(prop_len) as usize + prop_len
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        let start_len = buf.len();
        buf.put_u8(self.reason_code.into());

        let prop_len = var_int_len_from_size(size - 1);
        utils::write_variable_length(prop_len, buf);
        encode_property(&self.session_expiry_interval_secs, pt::SESS_EXPIRY_INT, buf)?;
        encode_property(&self.server_reference, pt::SERVER_REF, buf)?;
        encode_opt_props(
            &self.user_properties,
            &self.reason_string,
            buf,
            size - (buf.len() - start_len) as u32,
        )
    }
}
