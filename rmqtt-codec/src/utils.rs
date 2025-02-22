use std::{io::Cursor, num::NonZeroU16, num::NonZeroU32};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use crate::error::{DecodeError, EncodeError};
use crate::types::TimestampMillis;

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

macro_rules! prim_enum {
    (
        $( #[$enum_attr:meta] )*
        pub enum $name:ident {
            $(
                $( #[$enum_item_attr:meta] )*
                $var:ident=$val:expr
            ),+
        }) => {
        $( #[$enum_attr] )*
        #[repr(u8)]
        #[derive(Debug, Eq, PartialEq, Copy, Clone)]
        pub enum $name {
            $(
                $( #[$enum_item_attr] )*
                $var = $val
            ),+
        }
        impl std::convert::TryFrom<u8> for $name {
            type Error = $crate::error::DecodeError;
            fn try_from(v: u8) -> Result<Self, Self::Error> {
                match v {
                    $($val => Ok($name::$var)),+
                    ,_ => Err($crate::error::DecodeError::MalformedPacket)
                }
            }
        }
        // impl From<$name> for u8 {
        //     fn from(v: $name) -> Self {
        //         unsafe { ::std::mem::transmute(v) }
        //     }
        // }
    };
}

pub(crate) trait Decode: Sized {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError>;
}

pub(super) trait Property {
    fn read_value(&mut self, src: &mut Bytes) -> Result<(), DecodeError>;
}

impl<T: Decode> Property for Option<T> {
    fn read_value(&mut self, src: &mut Bytes) -> Result<(), DecodeError> {
        ensure!(self.is_none(), DecodeError::MalformedPacket); // property is set twice while not allowed
        *self = Some(T::decode(src)?);
        Ok(())
    }
}

impl Decode for bool {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.has_remaining(), DecodeError::InvalidLength); // expected more data within the field
        let v = src.get_u8();
        ensure!(v <= 0x1, DecodeError::MalformedPacket); // value is invalid
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
        ensure!(src.remaining() >= 4, DecodeError::InvalidLength); // expected more data within the field
        let val = src.get_u32();
        Ok(val)
    }
}

impl Decode for NonZeroU32 {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let val = NonZeroU32::new(u32::decode(src)?).ok_or(DecodeError::MalformedPacket)?;
        Ok(val)
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

pub(crate) fn take_properties(src: &mut Bytes) -> Result<Bytes, DecodeError> {
    let prop_len = decode_variable_length_cursor(src)?;
    ensure!(
        src.remaining() >= prop_len as usize,
        DecodeError::InvalidLength
    );

    Ok(src.split_to(prop_len as usize))
}

pub(crate) fn decode_variable_length(src: &[u8]) -> Result<Option<(u32, usize)>, DecodeError> {
    let mut cur = Cursor::new(src);
    match decode_variable_length_cursor(&mut cur) {
        Ok(len) => Ok(Some((len, cur.position() as usize))),
        Err(DecodeError::MalformedPacket) => Ok(None),
        Err(e) => Err(e),
    }
}

#[allow(clippy::cast_lossless)] // safe: allow cast through `as` because it is type-safe
pub(crate) fn decode_variable_length_cursor<B: Buf>(src: &mut B) -> Result<u32, DecodeError> {
    let mut shift: u32 = 0;
    let mut len: u32 = 0;
    loop {
        ensure!(src.has_remaining(), DecodeError::MalformedPacket);
        let val = src.get_u8();
        len += ((val & 0b0111_1111u8) as u32) << shift;
        if val & 0b1000_0000 == 0 {
            return Ok(len);
        } else {
            ensure!(shift < 21, DecodeError::InvalidLength);
            shift += 7;
        }
    }
}

pub(crate) trait Encode {
    fn encoded_size(&self) -> usize;

    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError>;
}
impl<T: Encode> Encode for Option<T> {
    fn encoded_size(&self) -> usize {
        if let Some(v) = self {
            v.encoded_size()
        } else {
            0
        }
    }
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        if let Some(v) = self {
            v.encode(buf)
        } else {
            Ok(())
        }
    }
}

impl Encode for bool {
    fn encoded_size(&self) -> usize {
        1
    }
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        if *self {
            buf.put_u8(0x1);
        } else {
            buf.put_u8(0x0);
        }
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
        2 + self.len()
    }
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        let len = u16::try_from(self.len()).map_err(|_| EncodeError::InvalidLength)?;
        buf.put_u16(len);
        buf.extend_from_slice(self.as_ref());
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

pub(crate) fn write_variable_length(len: u32, dst: &mut BytesMut) {
    match len {
        0..=127 => dst.put_u8(len as u8),
        128..=16_383 => {
            dst.put_slice(&[((len & 0b0111_1111) | 0b1000_0000) as u8, (len >> 7) as u8])
        }
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

#[inline]
pub(crate) fn timestamp_millis() -> TimestampMillis {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_variable_length() {
        fn assert_variable_length<B: AsRef<[u8]> + 'static>(bytes: B, res: (u32, usize)) {
            assert_eq!(decode_variable_length(bytes.as_ref()).unwrap(), Some(res));
        }

        assert_variable_length(b"\x7f\x7f", (127, 1));

        assert_eq!(decode_variable_length(b"\xff\xff\xff").unwrap(), None);

        assert_eq!(
            decode_variable_length(b"\xff\xff\xff\xff\xff\xff")
                .map_err(|e| matches!(e, DecodeError::InvalidLength)),
            Err(true)
        );

        assert_variable_length(b"\x00", (0, 1));
        assert_variable_length(b"\x7f", (127, 1));
        assert_variable_length(b"\x80\x01", (128, 2));
        assert_variable_length(b"\xff\x7f", (16383, 2));
        assert_variable_length(b"\x80\x80\x01", (16384, 3));
        assert_variable_length(b"\xff\xff\x7f", (2_097_151, 3));
        assert_variable_length(b"\x80\x80\x80\x01", (2_097_152, 4));
        assert_variable_length(b"\xff\xff\xff\x7f", (268_435_455, 4));
    }

    #[test]
    fn test_encode_variable_length() {
        let mut v = BytesMut::new();

        write_variable_length(123, &mut v);
        assert_eq!(v, [123].as_ref());

        v.clear();

        write_variable_length(129, &mut v);
        assert_eq!(v, b"\x81\x01".as_ref());

        v.clear();

        write_variable_length(16_383, &mut v);
        assert_eq!(v, b"\xff\x7f".as_ref());

        v.clear();

        write_variable_length(2_097_151, &mut v);
        assert_eq!(v, b"\xff\xff\x7f".as_ref());

        v.clear();

        write_variable_length(268_435_455, &mut v);
        assert_eq!(v, b"\xff\xff\xff\x7f".as_ref());

        // assert!(v.write_variable_length(MAX_VARIABLE_LENGTH + 1).is_err())
    }
}
