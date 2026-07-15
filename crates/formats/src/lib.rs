//! Read-only parsers for Rhythia file formats.
//!
//! Hard project rule: replay data is never written, re-encoded or modified.
//! This crate deliberately has no `.rhr` serializer and must never grow one;
//! the tool is a renderer, not an editor.

pub mod map;
pub mod rhr;
pub mod sspm;

mod reader;

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unexpected end of data at byte {at} (needed {wanted} more)")]
    UnexpectedEof { at: usize, wanted: usize },
    #[error("malformed varint string length at byte {at}")]
    BadStringLength { at: usize },
    #[error("string at byte {at} is not valid UTF-8")]
    InvalidUtf8 { at: usize },
    #[error("frame count {0} is not a valid length")]
    BadFrameCount(i64),
    #[error("map JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("map JSON: missing or invalid field {0:?}")]
    BadMapField(&'static str),
    #[error(".rhm archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error(".rhm archive has no \"map\" entry")]
    MissingMapEntry,
    #[error(".rhm entry {entry:?} is {declared} bytes, over the {limit}-byte limit")]
    ArchiveEntryTooLarge {
        entry: String,
        declared: u64,
        limit: u64,
    },
    #[error("unsupported map file extension: {0:?}")]
    UnsupportedExtension(String),
    #[error(".sspm map: {0}")]
    Malformed(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
