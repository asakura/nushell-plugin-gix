//! `gix open`: open a git repository, by default the one containing the current directory.

use crate::{cdt::Repository, git};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_protocol::{Category, LabeledError, Signature, Spanned, SyntaxShape, Type, Value};

use std::path::{Path, PathBuf};

use super::{super::GixPlugin, CommandError};

const REPO_FLAG: &str = "repo";

/// Opens the repository containing the current directory, or the one named by `--repo`.
#[derive(Clone, Copy, Debug)]
pub struct Open;

impl SimplePluginCommand for Open {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        "gix open"
    }

    fn description(&self) -> &'static str {
        "Open a git repository: the one containing the current directory, or the one at --repo."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                REPO_FLAG,
                SyntaxShape::Filepath,
                "open the repository at this path (relative to the current directory)",
                Some('r'),
            )
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

        let repo = call.get_flag::<Spanned<PathBuf>>(REPO_FLAG)?;

        run_open(&cwd, repo)
            .map(|repo| repo.into_value(call.head))
            .map_err(|e| e.into_labeled(call.head))
    }
}

/// Finds the repository for `repo` (resolved against `cwd`), or for `cwd` itself when no flag was given.
///
/// The returned handle only records locations; nothing is cached, so every later command
/// re-opens the repository. Keeping the opened repository in a plugin-side registry is left for issue #21.
fn run_open(cwd: &Path, repo: Option<Spanned<PathBuf>>) -> Result<Repository, CommandError> {
    match repo {
        None => Ok(git::discover(cwd)?.into()),
        Some(Spanned { item, span }) => git::discover(&cwd.join(&item))
            .map(Repository::from)
            .map_err(|source| CommandError::NotARepository {
                path: item,
                span,
                source,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::git::GixError;

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use nu_protocol::Span;
    use std::{fs, io};

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Io(#[from] io::Error),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Command(#[from] CommandError),

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

    fn flag(path: &Path) -> Spanned<PathBuf> {
        Spanned {
            item: path.to_owned(),
            span: Span::new(5, 9),
        }
    }

    mod run_open {
        use super::*;

        #[rstest]
        fn returns_handle_for_repository(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let handle = run_open(dir.path(), None)?;

            assert_eq!(handle.path.canonicalize()?, dir.path().canonicalize()?);
            assert_eq!(
                handle
                    .work_tree
                    .as_deref()
                    .map(Path::canonicalize)
                    .transpose()?,
                Some(dir.path().canonicalize()?)
            );

            Ok(())
        }

        #[rstest]
        fn discovers_parent_from_subdirectory(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let sub = dir.path().join("a").join("b");
            fs::create_dir_all(&sub)?;

            let handle = run_open(&sub, None)?;

            assert_eq!(handle.path.canonicalize()?, dir.path().canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn opens_absolute_repo_flag_from_any_directory(
            repo_dir: Result<TempDir, TestError>,
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let (repo, elsewhere) = (repo_dir?, dir?);

            let handle = run_open(elsewhere.path(), Some(flag(repo.path())))?;

            assert_eq!(handle.path.canonicalize()?, repo.path().canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn resolves_relative_repo_flag_against_cwd(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let nested = dir.path().join("nested");
            gix::init(&nested)?;

            let handle = run_open(dir.path(), Some(flag(Path::new("nested"))))?;

            assert_eq!(handle.path.canonicalize()?, nested.canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn fails_outside_a_repository(dir: Result<TempDir, TestError>) -> Result<(), TestError> {
            let dir = dir?;

            let err = run_open(dir.path(), None)
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(matches!(err, CommandError::Git(GixError::Open { .. })));

            Ok(())
        }

        #[rstest]
        fn reports_flag_span_for_non_repository(
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            let err = run_open(dir.path(), Some(flag(Path::new("missing"))))
                .err()
                .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

            assert!(
                matches!(&err, CommandError::NotARepository { span, .. } if *span == Span::new(5, 9)),
                "got {err:?}"
            );

            Ok(())
        }
    }
}
