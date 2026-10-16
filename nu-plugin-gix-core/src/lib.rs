//! Interface to git repositories via `gix`; must not depend on `nu_protocol` or `nu_plugin`.

#![cfg_attr(
    test,
    expect(
        clippy::panic_in_result_fn,
        reason = "rstest cases return Result for ?-based setup but use assertions for checks"
    ),
    expect(
        clippy::rest_pattern_accessible_field,
        reason = "tests match on a variant without caring about its fields"
    )
)]

mod error;
mod log;
mod open;
mod registry;
mod side;

pub use self::{
    error::GixError,
    log::{CommitInfo, log},
    open::{RepositoryLocation, discover},
    registry::{OpenRepository, Registry},
    side::{ResolvedRevision, Side},
};
