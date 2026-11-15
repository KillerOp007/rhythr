//! Little-endian byte cursor for the binary formats.

use crate::{Error, Result};

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Bytes consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Error::UnexpectedEof {
                at: self.pos,
                wanted: n - self.remaining(),
            });
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    /// A Godot `store_line` string: UTF-8 bytes up to a newline, which is
    /// consumed; a carriage return before it is dropped like `get_line`
    /// does.
    pub fn line(&mut self) -> Result<String> {
        let start = self.pos;
        let rest = &self.data[self.pos..];
        let Some(len) = rest.iter().position(|&b| b == b'\n') else {
            return Err(Error::UnexpectedEof {
                at: self.data.len(),
                wanted: 1,
            });
        };
        let bytes = self.take(len + 1)?;
        let text = bytes[..len].strip_suffix(b"\r").unwrap_or(&bytes[..len]);
        String::from_utf8(text.to_vec()).map_err(|_| Error::InvalidUtf8 { at: start })
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    /// A string stored as its UTF-8 bytes behind a count of UTF-16 code
    /// units (a C# `string.Length`), which is how the free client writes
    /// its `.phxr` strings. The two only agree for ASCII, so the bytes are
    /// walked character by character until the count is used up.
    pub fn utf16_counted_string(&mut self, units: usize) -> Result<String> {
        let start = self.pos;
        let rest = &self.data[self.pos..];
        let (mut len, mut seen) = (0usize, 0usize);
        while seen < units {
            let Some(&lead) = rest.get(len) else {
                return Err(Error::UnexpectedEof {
                    at: self.data.len(),
                    wanted: 1,
                });
            };
            let (bytes, counts) = match lead {
                0x00..=0x7F => (1, 1),
                0xC0..=0xDF => (2, 1),
                0xE0..=0xEF => (3, 1),
                0xF0..=0xF7 => (4, 2),
                _ => return Err(Error::InvalidUtf8 { at: start }),
            };
            len += bytes;
            seen += counts;
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidUtf8 { at: start })
    }

    /// A .NET `BinaryWriter` string: `Write7BitEncodedInt` length prefix
    /// (LEB128 varint, at most 5 bytes) followed by UTF-8 bytes.
    pub fn string(&mut self) -> Result<String> {
        let start = self.pos;
        let mut len: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            len |= u64::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift >= 35 {
                return Err(Error::BadStringLength { at: start });
            }
        }
        let bytes = self.take(len as usize)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::InvalidUtf8 { at: start })
    }
}
