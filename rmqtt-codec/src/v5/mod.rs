//! MQTT v5 Protocol codec

use std::num::NonZeroU16;

use bytestring::ByteString;
use nonzero_ext::nonzero;

mod codec;
mod decode;
mod encode;
mod packet;

pub use self::codec::Codec;
pub use self::packet::*;

/// A key-value pair representing a single user property
pub type UserProperty = (ByteString, ByteString);
/// A list of user properties for MQTT v5 packets
pub type UserProperties = Vec<UserProperty>;

/// Default maximum receive value (65535)
pub(crate) const RECEIVE_MAX_DEFAULT: NonZeroU16 = nonzero!(65535u16);
