//! `gix open`: open a git repository, by default the one containing the current directory.

use crate::{
    cdt::Repository,
    git::{self, Registry},
};

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
        plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        let cwd = PathBuf::from(engine.get_current_dir()?);
        let repo = call.get_flag::<Spanned<PathBuf>>(REPO_FLAG)?;

        run_open(plugin.registry(), &cwd, repo)
            .map(|repo| repo.into_value(call.head))
            .map_err(|e| e.into_labeled(call.head))
    }
}

/// Finds the repository for `repo` (resolved against `cwd`), or for `cwd` itself when no flag was given.
///
/// The repository is opened once and cached in `registry`; the returned handle carries the
/// cache key plus the on-disk locations needed to rebuild it after a plugin restart.
fn run_open(
    registry: &Registry,
    cwd: &Path,
    repo: Option<Spanned<PathBuf>>,
) -> Result<Repository, CommandError> {
    let found = match repo {
        None => git::discover(cwd)?,
        Some(Spanned { item, span }) => {
            git::discover(&cwd.join(&item)).map_err(|source| CommandError::NotARepository {
                path: item,
                span,
                source,
            })?
        }
    };

    let id = registry.register(&found)?;

    Ok(Repository::new(id, found))
}

#[cfg(test)]
mod tests {
    use super::*;

    use nu_protocol::Span;
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
        fn delegates_to_git_discover(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let handle = run_open(&Registry::new(), dir.path(), None)?;

            assert_eq!(handle.path.canonicalize()?, dir.path().canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn caches_the_opened_repository(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let registry = Registry::new();
            let handle = run_open(&registry, dir.path(), None)?;

            registry.release(handle.id);

            let err = registry.acquire(handle.id, &handle.location());

            assert!(err.is_ok(), "rebuild after release failed: {err:?}");

            Ok(())
        }

        #[rstest]
        fn resolves_relative_repo_flag_against_cwd(
            repo_dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = repo_dir?;
            let nested = dir.path().join("nested");

            gix::init(&nested)?;

            let handle = run_open(
                &Registry::new(),
                dir.path(),
                Some(flag(Path::new("nested"))),
            )?;

            assert_eq!(handle.path.canonicalize()?, nested.canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn reports_flag_span_for_non_repository(
            dir: Result<TempDir, TestError>,
        ) -> Result<(), TestError> {
            let dir = dir?;

            let err = run_open(
                &Registry::new(),
                dir.path(),
                Some(flag(Path::new("missing"))),
            )
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
