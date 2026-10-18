//! Plugin-side test fixtures: binds the generic helpers in `nu_plugin_gix_testing_support` to this
//! crate's [`GixPlugin`].

use crate::GixPlugin;

use nu_plugin_gix_testing_support::{FixtureError, plugin_test_for};
use nu_plugin_test_support::PluginTest;
use rstest::fixture;

use std::sync::Arc;

/// Name the test plugin is registered under.
const PLUGIN_NAME: &str = "nu_plugin_gix";

/// A fresh in-process [`PluginTest`] over a default [`GixPlugin`], with Nushell's `describe` available.
///
/// A new plugin instance per test keeps the registry cache isolated between tests.
#[fixture]
pub fn plugin_test() -> Result<PluginTest, FixtureError> {
    plugin_test_for(PLUGIN_NAME, Arc::new(GixPlugin::default()))
}
