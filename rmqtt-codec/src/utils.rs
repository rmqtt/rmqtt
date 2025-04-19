use std::{io::Cursor, num::NonZeroU16, num::NonZeroU32};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use crate::error::{DecodeError, EncodeError};

/// Helper macro for early error return on condition failure
macro_rules! ensure {
    ($cond:expr, $e:expr) => {
        if !($cond) {
            return Err($e);
        }
    };
    ($cond:expr, $fmt:expr, $($arg:tt)+) => {
        if !($cond) {
            return Err($fmt, $($arg)+);
        }
    };
}

/// Macro for creating primitive enums with u8 conversion
macro_rules! prim_enum {
    ($( #[$enum_attr:meta] )* pub enum $name:ident { $($( #[$enum_item_attr:meta] )* $var:ident=$val:expr ),+ }) => {
        $( #[$enum_attr] )*
        #[repr(u8)]
        #[derive(Debug, Eq, PartialEq, Copy, Clone)]
        pub enum $name {
            $($( #[$enum_item_attr] )* $var = $val ),+
        }
        impl std::convert::TryFrom<u8> for $name {
            type Error = $crate::error::DecodeError;
            fn try_from(v: u8) -> Result<Self, Self::Error> {
                match v {
                    $($val => Ok($name::$var)),+ ,
                    _ => Err($crate::error::DecodeError::MalformedPacket)
                }
            }
        }
    };
}

/// Trait for decoding types from network bytes
pub(crate) trait Decode: Sized {
    /// Decodes from byte buffer, advancing position
    ///
    /// # Errors
    /// Returns `DecodeError` on invalid data or insufficient bytes
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError>;
}

/// Trait for reading MQTT properties with validation
pub(super) trait Property {
    /// Reads property value from buffer, ensuring single assignment
    fn read_value(&mut self, src: &mut Bytes) -> Result<(), DecodeError>;
}

impl<T: Decode> Property for Option<T> {
    fn read_value(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        ensure!(self.is_none(), DecodeError::MalformedPacket);
        *self = Some(T::decode(src)?);
        Ok(())
    }
}

// Primitive type decoding implementations
impl Decode for bool {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.has_remaining(), DecodeError::InvalidLength);
        let v = src.get_u8();
        ensure!(v <= 0x1, DecodeError::MalformedPacket);
        Ok(v == 0x1)
    }
}

impl Decode for u16 {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.remaining() >= 2, DecodeError::InvalidLength);
        Ok(src.get_u16())
    }
}

impl Decode for u32 {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.remaining() >= 4, DecodeError::InvalidLength);
        Ok(src.get_u32())
    }
}

impl Decode for NonZeroU32 {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        NonZeroU32::new(u32::decode(src)?).ok_or(DecodeError::MalformedPacket)
    }
}

impl Decode for NonZeroU16 {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        NonZeroU16::new(u16::decode(src)?).ok_or(DecodeError::MalformedPacket)
    }
}

impl Decode for Bytes {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let len = u16::decode(src)? as usize;
        ensure!(src.remaining() >= len, DecodeError::InvalidLength);
        Ok(src.split_to(len))
    }
}

impl Decode for ByteString {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ByteString::try_from(Bytes::decode(src)?).map_err(|_| DecodeError::Utf8Error)
    }
}

/// Extracts property bytes with length prefix
pub(crate) fn take_properties(src: &mut Bytes) -> Result<Bytes, DecodeError> {
    let prop_len = decode_variable_length_cursor(src)?;
    ensure!(src.remaining() >= prop_len as usize, DecodeError::InvalidLength);
    Ok(src.split_to(prop_len as usize))
}

/// Decodes MQTT variable-length integer from byte slice
///
/// # Returns
/// (decoded value, bytes consumed) or None if incomplete
pub(crate) fn decode_variable_length(src: &[u8]) -> Result<Option<(u32, usize)>, DecodeError> {
    let mut cur = Cursor::new(src);
    match decode_variable_length_cursor(&mut cur) {
        Ok(len) => Ok(Some((len, cur.position() as usize))),
        Err(DecodeError::MalformedPacket) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Decodes variable-length integer from buffer
///
/// # MQTT Spec
/// Variable length integers use up to 4 bytes with continuation bits
/// Max value: 268,435,455 (0xFFFFFF7F)
#[allow(clippy::cast_lossless)]
pub(crate) fn decode_variable_length_cursor<B: Buf>(src: &mut B) -> Result<u32, DecodeError> {
    let mut shift: u32 = 0;
    let mut len: u32 = 0;
    loop {
        ensure!(src.has_remaining(), DecodeError::MalformedPacket);
        let val = src.get_u8();
        len += ((val & 0b0111_1111) as u32) << shift;
        if val & 0b1000_0000 == 0 {
            return Ok(len);
        }
        ensure!(shift < 21, DecodeError::InvalidLength);
        shift += 7;
    }
}

/// Trait for encoding types to network bytes
pub(crate) trait Encode {
    /// Calculates serialized size in bytes
    fn encoded_size(&self) -> usize;

    /// Writes serialized data to buffer
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError>;
}

// Encoding implementations for optional values
impl<T: Encode> Encode for Option<T> {
    fn encoded_size(&self) -> usize {
        self.as_ref().map_or(0, |v| v.encoded_size())
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        if let Some(v) = self {
            v.encode(buf)
        } else {
            Ok(())
        }
    }
}

// Primitive type encoding implementations
impl Encode for bool {
    fn encoded_size(&self) -> usize {
        1
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        buf.put_u8(u8::from(*self));
        Ok(())
    }
}

impl Encode for u16 {
    fn encoded_size(&self) -> usize {
        2
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        buf.put_u16(*self);
        Ok(())
    }
}

impl Encode for NonZeroU16 {
    fn encoded_size(&self) -> usize {
        2
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        self.get().encode(buf)
    }
}

impl Encode for u32 {
    fn encoded_size(&self) -> usize {
        4
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        buf.put_u32(*self);
        Ok(())
    }
}

impl Encode for NonZeroU32 {
    fn encoded_size(&self) -> usize {
        4
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        self.get().encode(buf)
    }
}

impl Encode for Bytes {
    fn encoded_size(&self) -> usize {
        2 + self.len() // Length prefix + data
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let len = u16::try_from(self.len()).map_err(|_| EncodeError::InvalidLength)?;
        buf.put_u16(len);
        buf.extend_from_slice(self);
        Ok(())
    }
}

impl Encode for ByteString {
    fn encoded_size(&self) -> usize {
        self.as_bytes().encoded_size()
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        self.as_bytes().encode(buf)
    }
}

impl Encode for (ByteString, ByteString) {
    fn encoded_size(&self) -> usize {
        self.0.encoded_size() + self.1.encoded_size()
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        self.0.encode(buf)?;
        self.1.encode(buf)
    }
}

impl Encode for &[u8] {
    fn encoded_size(&self) -> usize {
        2 + self.len()
    }

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let len = u16::try_from(self.len()).map_err(|_| EncodeError::InvalidLength)?;
        buf.put_u16(len);
        buf.extend_from_slice(self);
        Ok(())
    }
}

/// Writes MQTT variable-length integer to buffer
///
/// # Panics
/// For values exceeding 268,435,455 (should be validated earlier)
pub(crate) fn write_variable_length(len: u32, dst: &mut BytesMut) {
    match len {
        0..=127 => dst.put_u8(len as u8),
        128..=16_383 => dst.put_slice(&[((len & 0b0111_1111) | 0b1000_0000) as u8, (len >> 7) as u8]),
        16_384..=2_097_151 => {
            dst.put_slice(&[
                ((len & 0b0111_1111) | 0b1000_0000) as u8,
                (((len >> 7) & 0b0111_1111) | 0b1000_0000) as u8,
                (len >> 14) as u8,
            ]);
        }
        2_097_152..=268_435_455 => {
            dst.put_slice(&[
                ((len & 0b0111_1111) | 0b1000_0000) as u8,
                (((len >> 7) & 0b0111_1111) | 0b1000_0000) as u8,
                (((len >> 14) & 0b0111_1111) | 0b1000_0000) as u8,
                (len >> 21) as u8,
            ]);
        }
        _ => panic!("length is too big"), // todo: verify at higher level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_decoding() {
        // Test valid decodes
        assert_eq!(decode_variable_length(&[0x00]).unwrap(), Some((0, 1)));
        assert_eq!(decode_variable_length(&[0x7F]).unwrap(), Some((127, 1)));
        assert_eq!(decode_variable_length(&[0x80, 0x01]).unwrap(), Some((128, 2)));

        // Test incomplete data
        assert_eq!(decode_variable_length(&[0x80]).unwrap(), None);

        // Test invalid length
        let res = decode_variable_length(&[0xFF, 0xFF, 0xFF, 0xFF]);
        assert!(matches!(res, Err(DecodeError::InvalidLength)));
    }

    #[test]
    fn variable_length_encoding() {
        let mut buf = BytesMut::new();

        write_variable_length(127, &mut buf);
        assert_eq!(buf.as_ref(), &[0x7F]);

        buf.clear();
        write_variable_length(16_383, &mut buf);
        assert_eq!(buf.as_ref(), &[0xFF, 0x7F]);

        buf.clear();
        write_variable_length(268_435_455, &mut buf);
        assert_eq!(buf.as_ref(), &[0xFF, 0xFF, 0xFF, 0x7F]);
    }
}
