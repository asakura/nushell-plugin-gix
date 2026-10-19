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
#[cfg(test)]
mod fixtures;
mod log;
mod refs;
mod registry;
mod repository;
mod rev;
mod side;

pub use self::{
    error::GixError,
    log::{CommitInfo, log},
    refs::{HeadInfo, RefExpect, RefInfo, RefTarget, head, list, set},
    registry::Registry,
    repository::{OpenRepository, RepositoryLocation},
    rev::{RevInfo, RevKind, resolve},
    side::{ResolvedRevision, Side},
};
