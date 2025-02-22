//! MQTT v5 Protocol codec

use std::num::NonZeroU16;

use bytestring::ByteString;
use nonzero_ext::nonzero;

// #[allow(clippy::module_inception)]
mod codec;
mod decode;
mod encode;
mod packet;

pub use self::codec::Codec;
pub use self::packet::*;

pub type UserProperty = (ByteString, ByteString);
pub type UserProperties = Vec<UserProperty>;

const RECEIVE_MAX_DEFAULT: NonZeroU16 = nonzero!(65535u16);
