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

mod config;
mod error;
#[cfg(test)]
mod fixtures;
mod log;
mod refs;
mod registry;
mod repository;
mod rev;
mod side;

pub use gix::pathspec::Pattern;

pub use self::{
    config::{
        Config, ConfigKey, ConfigType, ConfigValue, GroupPath, LookupPath, Occurrence, Origin,
        PathContext, RawValue, ScalarType, SectionName, Subsection, VariableName, convert,
        convert_path,
    },
    error::{GixError, SectionFault},
    log::{
        CommitInfo, CommitLog, FileChange, FileChangeKind, FileStat, LogSettings, LogStart,
        Mailmap, SignatureInfo, log, pathspec,
    },
    refs::{HeadInfo, RefExpect, RefInfo, RefTarget, head, list, set},
    registry::Registry,
    repository::{OpenRepository, RepositoryLocation},
    rev::{RevInfo, RevKind, resolve},
    side::{ResolvedRevision, Side},
};
