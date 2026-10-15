//! Custom Nushell data types: plain serializable handles with no `gix` types.

mod handle;
mod repository;

pub use self::{
    handle::{Handle, downcast},
    repository::Repository,
};
