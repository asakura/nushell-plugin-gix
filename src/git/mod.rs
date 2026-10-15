//! Interface to git repositories via `gix`; must not depend on `nu_protocol` or `nu_plugin`.

mod error;
mod log;
mod open;
mod registry;

pub use self::{
    error::GixError,
    log::{CommitInfo, log},
    open::{RepositoryLocation, discover},
    registry::{Registry, Snapshot},
};
