//! The files whose entries differ between two trees, and how each one changed.
//!
//! `diff` walks the trees and `types` holds the result. Rename tracking is off, so a moved file is
//! one deletion and one addition, and with line counts it counts in full.

mod diff;
mod types;

pub(super) use self::diff::{changed_files, changed_files_between, for_each_change};
pub use self::types::{FileChange, FileChangeKind, FileStat};
