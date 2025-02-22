use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use crate::error::{DecodeError, EncodeError};
use crate::utils::{self, Decode, Property};
use crate::v5::{encode::*, property_type as pt, UserProperties, UserProperty};

/// AUTH message
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Auth {
    pub reason_code: AuthReasonCode,
    pub auth_method: Option<ByteString>,
    pub auth_data: Option<Bytes>,
    pub reason_string: Option<ByteString>,
    pub user_properties: UserProperties,
}

prim_enum! {
    /// AUTH reason codes
    pub enum AuthReasonCode {
        Success = 0,
        ContinueAuth = 24,
        ReAuth = 25
    }
}

impl From<AuthReasonCode> for u8 {
    fn from(v: AuthReasonCode) -> Self {
        match v {
            AuthReasonCode::Success => 0,
            AuthReasonCode::ContinueAuth => 24,
            AuthReasonCode::ReAuth => 25,
        }
    }
}

impl Auth {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let auth = if src.has_remaining() {
            let reason_code = src.get_u8().try_into()?;

            if src.has_remaining() {
                let mut auth_method = None;
                let mut auth_data = None;
                let mut reason_string = None;
                let mut user_properties = Vec::new();

                if reason_code != AuthReasonCode::Success || src.has_remaining() {
                    let prop_src = &mut utils::take_properties(src)?;
                    while prop_src.has_remaining() {
                        match prop_src.get_u8() {
                            pt::AUTH_METHOD => auth_method.read_value(prop_src)?,
                            pt::AUTH_DATA => auth_data.read_value(prop_src)?,
                            pt::REASON_STRING => reason_string.read_value(prop_src)?,
                            pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
                            _ => return Err(DecodeError::MalformedPacket),
                        }
                    }
                    ensure!(!src.has_remaining(), DecodeError::InvalidLength);
                }

                Self {
                    reason_code,
                    auth_method,
                    auth_data,
                    reason_string,
                    user_properties,
                }
            } else {
                Self {
                    reason_code,
                    ..Default::default()
                }
            }
        } else {
            Self::default()
        };
        Ok(auth)
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            reason_code: AuthReasonCode::Success,
            auth_method: None,
            auth_data: None,
            reason_string: None,
            user_properties: Vec::new(),
        }
    }
}

impl EncodeLtd for Auth {
    fn encoded_size(&self, limit: u32) -> usize {
        const HEADER_LEN: usize = 1; // reason code

        let mut prop_len =
            encoded_property_size(&self.auth_method) + encoded_property_size(&self.auth_data);
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
        encode_property(&self.auth_method, pt::AUTH_METHOD, buf)?;
        encode_property(&self.auth_data, pt::AUTH_DATA, buf)?;
        encode_opt_props(
            &self.user_properties,
            &self.reason_string,
            buf,
            size - (buf.len() - start_len) as u32,
        )
    }
}
