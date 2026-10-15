//! Custom Nushell data types: plain serializable handles with no `gix` types.

mod changeset;
mod handle;
mod index;
mod repository;

#[expect(
    unused_imports,
    reason = "Index and Changeset have no producing command until the `gix index` and `gix changes` issues land"
)]
pub use self::{
    changeset::{Change, ChangeKind, Changeset, Side},
    index::Index,
};
pub use self::{
    handle::{Handle, downcast},
    repository::Repository,
};
