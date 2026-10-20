//! One commit of the history as plain data: its ids, its message in parts, and its signatures.
//!
//! `info` builds a `CommitInfo` from a `gix` commit, `signature` maps and converts the two signatures,
//! and `decoder` decodes text in the encoding the commit declares. An unknown encoding falls back to
//! UTF-8 and malformed bytes become U+FFFD, so a commit with bad text still lists.

mod decoder;
mod info;
mod signature;

pub use self::{info::CommitInfo, signature::SignatureInfo};
