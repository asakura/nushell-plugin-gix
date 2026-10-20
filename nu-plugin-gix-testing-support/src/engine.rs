//! In-process plugin-engine helpers: plugin registration, custom-value input and `PWD` pinning.

use crate::error::FixtureError;

use nu_cmd_lang::Describe;
use nu_command::First;
use nu_plugin::Plugin;
use nu_plugin_test_support::PluginTest;
use nu_protocol::{CustomValue, IntoPipelineData as _, PipelineData, Span, Value};

use std::{path::Path, sync::Arc};

const PWD_VAR: &str = "PWD";

/// A path that never holds a repository, used to provoke a real `gix` failure.
const MISSING_REPOSITORY: &str = "/nonexistent/nu-plugin-gix-fixture";

/// A fresh in-process [`PluginTest`] with `plugin` registered as `name` and Nushell's `describe`
/// available.
///
/// Callers pass a new plugin instance per test to keep each registry cache isolated.
///
/// # Errors
///
/// Returns [`FixtureError::Failure`] when the engine rejects the plugin or the `describe` command.
pub fn plugin_test_for<P>(name: &str, plugin: Arc<P>) -> Result<PluginTest, FixtureError>
where
    P: Plugin + Send + 'static,
{
    let mut test = PluginTest::new(name, plugin)
        .map_err(|error| FixtureError::Failure(format!("plugin registration failed: {error}")))?;

    test.add_decl(Box::new(Describe))
        .map_err(|error| FixtureError::Failure(format!("describe registration failed: {error}")))?;

    test.add_decl(Box::new(First))
        .map_err(|error| FixtureError::Failure(format!("first registration failed: {error}")))?;

    Ok(test)
}

/// A fresh in-process [`PluginTest`] over a default-constructed `P` registered as `name`.
///
/// Shorthand for [`plugin_test_for`] with `Arc::new(P::default())`, giving each test its own plugin instance.
///
/// # Errors
///
/// Returns [`FixtureError::Failure`] when the engine rejects the plugin or the `describe` command.
pub fn default_plugin_test<P>(name: &str) -> Result<PluginTest, FixtureError>
where
    P: Plugin + Default + Send + 'static,
{
    plugin_test_for(name, Arc::new(P::default()))
}

/// Pipeline input carrying `value` as a custom value, as a command receives it from the engine.
#[must_use = "returns the pipeline input; discarding it runs nothing"]
pub fn custom_value_input<V>(value: V) -> PipelineData
where
    V: CustomValue + 'static,
{
    Value::test_custom_value(Box::new(value)).into_pipeline_data()
}

/// Pipeline input holding the custom value built from the fixture directory `dir`.
///
/// `discover` turns `dir` into whatever location type `build` needs, and `build` makes the custom value from it, so
/// the helper works with any handle and any error type.
///
/// # Errors
///
/// Returns the error of `discover` unchanged.
pub fn discovered_input<D, B, L, V, E>(dir: &Path, discover: D, build: B) -> Result<PipelineData, E>
where
    D: FnOnce(&Path) -> Result<L, E>,
    B: FnOnce(L) -> V,
    V: CustomValue + 'static,
{
    Ok(custom_value_input(build(discover(dir)?)))
}

/// Pins `$env.PWD` of `test` to `dir`: the bare test engine has no working directory, and
/// commands such as `gix open` resolve their paths against it.
pub fn pin_pwd<P>(test: &mut PluginTest, dir: P)
where
    P: AsRef<Path>,
{
    test.engine_state_mut().add_env_var(
        PWD_VAR.to_owned(),
        Value::string(dir.as_ref().to_string_lossy(), Span::test_data()),
    );
}

/// A real [`gix::Error`], as `gix::open` reports it for a path that does not exist.
///
/// # Errors
///
/// Returns [`FixtureError::Failure`] when the path unexpectedly opens as a repository.
pub fn gix_error() -> Result<gix::Error, FixtureError> {
    gix::open(MISSING_REPOSITORY).err().ok_or_else(|| {
        FixtureError::Failure(format!("`{MISSING_REPOSITORY}` opened as a repository"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;
    use std::io;

    #[derive(thiserror::Error, Debug)]
    enum TestError {
        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        #[expect(
            dead_code,
            reason = "every test error carries a Failure variant, even where no test builds one"
        )]
        Failure(String),
    }

    mod gix_error_helper {
        use super::*;

        #[rstest]
        fn returns_an_error_for_a_missing_path() -> Result<(), TestError> {
            let error = gix_error()?;

            assert_matches!(error.to_string().is_empty(), false);

            Ok(())
        }
    }
}
