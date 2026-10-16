//! Custom Nushell data types: plain serializable handles with no `gix` types.

mod changeset;
mod handle;
mod index;
mod repository;

pub use self::{
    changeset::{Change, ChangeKind, Changeset},
    index::Index,
};
pub use self::{
    handle::{Handle, downcast},
    repository::Repository,
};
