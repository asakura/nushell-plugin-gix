//! Resolves pathspecs written relative to a directory into pathspecs relative to the repository root, and
//! builds the filter that keeps the commits changing them.
//!
//! - `spec` runs first, on the caller's side: `pathspec` turns what the user typed into a `Pattern`
//!   anchored at the repository root.
//! - `filter` runs during the walk: `PathFilter` decides, per commit, whether it is listed and which
//!   parent is followed.

mod filter;
mod spec;

pub(super) use self::filter::PathFilter;
pub use self::spec::pathspec;
