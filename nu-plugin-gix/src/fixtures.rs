//! Plugin-side test fixtures: binds the generic helpers in `nu_plugin_gix_testing_support` to this
//! crate's [`GixPlugin`].

use crate::GixPlugin;
use crate::cdt::Repository;

use nu_plugin_gix_core::{GixError, RepositoryLocation};
use nu_plugin_gix_testing_support::{FixtureError, custom_value_input, plugin_test_for};
use nu_plugin_test_support::PluginTest;
use nu_protocol::PipelineData;
use rstest::fixture;

use std::{path::Path, sync::Arc};

/// Registry id given to the handle built by [`repo_input`].
const FIXTURE_HANDLE_ID: u64 = 1;

/// Name the test plugin is registered under.
const PLUGIN_NAME: &str = "nu_plugin_gix";

/// A fresh in-process [`PluginTest`] over a default [`GixPlugin`], with Nushell's `describe` available.
///
/// A new plugin instance per test keeps the registry cache isolated between tests.
#[fixture]
pub fn plugin_test() -> Result<PluginTest, FixtureError> {
    plugin_test_for(PLUGIN_NAME, Arc::new(GixPlugin::default()))
}

/// Errors raised while building the pipeline input for a fixture repository.
#[derive(Debug, thiserror::Error)]
pub enum RepoInputError {
    /// The fixture directory is not a repository, or its repository cannot be opened.
    #[error(transparent)]
    Discover(#[from] GixError),
}

/// Pipeline input holding the repository handle for the fixture repository at `dir`.
///
/// # Errors
///
/// Returns [`RepoInputError::Discover`] when `dir` does not hold a repository that can be opened.
#[must_use = "returns the pipeline input or the discovery error; discarding it runs nothing"]
pub fn repo_input(dir: &Path) -> Result<PipelineData, RepoInputError> {
    let location = RepositoryLocation::discover(dir)?;

    Ok(custom_value_input(Repository::new(
        FIXTURE_HANDLE_ID,
        location,
    )))
}
