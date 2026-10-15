//! `gix open`: open the git repository in the current directory.

use crate::{error::GixError, git, repository::GixRepository};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, LabeledError, Signature, Type, Value};

use std::path::PathBuf;

use super::super::GixPlugin;

/// Opens the repository in the shell's current directory.
#[derive(Clone, Copy, Debug)]
pub struct Open;

impl SimplePluginCommand for Open {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        "gix open"
    }

    fn description(&self) -> &'static str {
        "Open the git repository in the current directory."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_type(Type::Nothing, Type::Custom("git repository".into()))
            .category(Category::Custom("gix".into()))
    }

    fn run(
        &self,
        _plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        let cwd = PathBuf::from(engine.get_current_dir()?);

        run_open(cwd)
            .map(|repo| repo.into_value(call.head))
            .map_err(|e| e.into_labeled(call.head))
    }
}

/// Validates that `path` is a repository and returns a handle to it.
fn run_open(path: PathBuf) -> Result<GixRepository, GixError> {
    git::open(&path)?;
    Ok(GixRepository { path })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use std::io;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    #[fixture]
    fn dir() -> Result<TempDir, TestError> {
        Ok(TempDir::new()?)
    }

    /// A temporary directory holding a freshly initialised git repository.
    #[fixture]
    fn repo_dir(dir: Result<TempDir, TestError>) -> Result<TempDir, TestError> {
        let dir = dir?;
        gix::init(dir.path())?;

        Ok(dir)
    }

    mod run_open {
        use super::*;

        #[rstest]
        fn returns_handle_for_repository(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let handle = run_open(dir.path().to_owned())?;

            assert_eq!(handle.path, dir.path());

            Ok(())
        }

        #[rstest]
        fn fails_outside_a_repository(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            let err = run_open(dir.path().to_owned())
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, GixError::Open { .. }));

            Ok(())
        }
    }
}
