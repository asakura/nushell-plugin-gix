//! Custom Nushell data types: plain serializable handles with no repository-bound `gix` types
//! (only `ObjectId` and `Spec`, see `.claude/rules/plugin.md`, section `gix_exceptions`).

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
