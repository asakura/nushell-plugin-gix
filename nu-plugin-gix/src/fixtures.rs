//! Plugin-side test fixtures: binds the generic helpers in `nu_plugin_gix_testing_support` to this
//! crate's [`GixPlugin`] and [`Repository`] types.

use crate::GixPlugin;
use crate::cdt::Repository;

use nu_plugin_gix_core::{GixError, RepositoryLocation};
use nu_plugin_gix_testing_support::{FixtureError, default_plugin_test, discovered_input};
use nu_plugin_test_support::PluginTest;
use nu_protocol::PipelineData;
use rstest::fixture;

use std::path::Path;

/// Registry id given to the handle built by [`repo_input`].
const FIXTURE_HANDLE_ID: u64 = 1;

/// Name the test plugin is registered under.
const PLUGIN_NAME: &str = "nu_plugin_gix";

/// Errors raised while building the pipeline input for a fixture repository.
#[derive(Debug, thiserror::Error)]
pub enum RepoInputError {
    /// The fixture directory is not a repository, or its repository cannot be opened.
    #[error(transparent)]
    Discover(#[from] GixError),
}

/// A fresh in-process [`PluginTest`] over a default [`GixPlugin`], with Nushell's `describe` available.
///
/// A new plugin instance per test keeps the registry cache isolated between tests.
#[fixture]
pub fn plugin_test() -> Result<PluginTest, FixtureError> {
    default_plugin_test::<GixPlugin>(PLUGIN_NAME)
}

/// Pipeline input holding the repository handle for the fixture repository at `dir`.
///
/// The location is discovered exactly as `gix open` discovers it, so the handle matches production.
///
/// # Errors
///
/// Returns [`RepoInputError::Discover`] when `dir` does not hold a repository that can be opened.
#[must_use = "returns the pipeline input or the discovery error; discarding it runs nothing"]
pub fn repo_input(dir: &Path) -> Result<PipelineData, RepoInputError> {
    let input = discovered_input(dir, RepositoryLocation::discover, |location| {
        Repository::new(FIXTURE_HANDLE_ID, location)
    })?;

    Ok(input)
}
