//! `gix open`: open a git repository, by default the one containing the current directory.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::CommandError;

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{GixError, Registry};
use nu_protocol::{Category, Example, LabeledError, Signature, Spanned, SyntaxShape, Type, Value};

use std::path::{Path, PathBuf};

const COMMAND_NAME: &str = "gix open";
const CATEGORY: &str = "gix";
const REPO_FLAG: &str = "repo";

/// Opens the repository containing the current directory, or the one named by `--repo`.
#[derive(Clone, Copy, Debug)]
pub struct Open;

impl SimplePluginCommand for Open {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        COMMAND_NAME
    }

    fn description(&self) -> &'static str {
        "Open the git repository that contains the current directory, or the one that contains --repo."
    }

    fn extra_description(&self) -> &'static str {
        "Error codes: gix::not-a-repository, gix::open-failed, gix::current-dir-unavailable, gix::invalid-argument"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open",
                description: "Open the repository that contains the current directory",
                result: None,
            },
            Example {
                example: "gix open --repo ~/src/project",
                description: "Open the repository that contains ~/src/project",
                result: None,
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .named(
                REPO_FLAG,
                SyntaxShape::Filepath,
                "open the repository containing this path (relative to the current directory)",
                Some('r'),
            )
            .input_output_type(
                Type::Nothing,
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
            )
            .category(Category::Custom(CATEGORY.into()))
    }

    fn run(
        &self,
        plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        open_call(plugin.registry(), engine, call)
            .map(|repository| repository.into_value(call.head))
            .map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the working directory and `--repo` from `call`, then opens the repository they name.
fn open_call(
    registry: &Registry,
    engine: &EngineInterface,
    call: &EvaluatedCall,
) -> Result<Repository, CommandError> {
    let cwd = engine
        .get_current_dir()
        .map_err(|source| CommandError::CurrentDir {
            source: Box::new(source),
            span: call.head,
        })?;

    let repo = call
        .get_flag::<Spanned<PathBuf>>(REPO_FLAG)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: call.get_flag_span(REPO_FLAG).unwrap_or(call.head),
        })?;

    run_open(registry, Path::new(&cwd), repo)
}

/// Finds the repository for `repo` (resolved against `cwd`), or for `cwd` itself when no flag was given.
///
/// Discovery opens the repository once, and `registry` caches that same opened repository; the
/// returned handle carries the cache key plus the on-disk locations needed to rebuild it after a
/// plugin restart. With `--repo`, a missing or unopenable repository is reported at the flag's span.
#[expect(
    clippy::rest_pattern_accessible_field,
    reason = "only the not-found and open-failed kinds carry the flag's span; other failures keep their own error"
)]
fn run_open(
    registry: &Registry,
    cwd: &Path,
    repo: Option<Spanned<PathBuf>>,
) -> Result<Repository, CommandError> {
    let (id, location) =
        match repo {
            None => registry.discover_and_register(cwd)?,
            Some(Spanned { item, span }) => registry
                .discover_and_register(&cwd.join(&item))
                .map_err(|source| match source {
                    GixError::NotFound { .. } => CommandError::NotARepository {
                        path: item,
                        span,
                        source,
                    },
                    GixError::Open { .. } => CommandError::OpenFailed {
                        path: item,
                        span,
                        source,
                    },
                    other => CommandError::Git(other),
                })?,
        };

    Ok(Repository::new(id, location))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::plugin_test;

    use miette::Diagnostic as _;
    use nu_plugin_gix_testing_support::{
        CorruptTestRepo, FixtureError, GIT_DIR, HEAD_FILE, TestRepo, UnbornTestRepo, bare_repo,
        corrupt_repo, pin_pwd, plain_dir, test_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::{ShellError, Span};
    use rstest::rstest;
    use tempfile::TempDir;

    use std::{assert_matches, fs, io, path::Path, sync::Arc};

    const OPEN_COMMAND: &str = "gix open";
    const OPEN_FLAG: &str = "gix open --repo";
    const DESCRIBE_COMMAND: &str = "describe";
    const DISCOVERY_LABEL: &str = "run this from inside a git repository";
    const SUBDIR: &str = "a/b";
    const NOT_A_REPOSITORY_CODE: &str = "gix::not-a-repository";
    const OPEN_FAILED_CODE: &str = "gix::open-failed";
    const REPOSITORY_TYPE_NAME: &str = <Repository as Handle>::TYPE_NAME;
    const EXPECTED_ERROR: &str = "expected an error";
    const FLAG_SPAN: Span = Span { start: 5, end: 9 };
    // Missing, so neither the path nor any of its parents holds a repository.
    const MISSING_DIR: &str = "missing";
    const NESTED_DIR: &str = "nested";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] ShellError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Command(#[from] CommandError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Gix(#[from] gix::Error),

        #[error(transparent)]
        Io(#[from] io::Error),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    fn flag(path: &Path) -> Spanned<PathBuf> {
        Spanned {
            item: path.to_owned(),
            span: FLAG_SPAN,
        }
    }

    mod eval {
        use super::*;

        /// Runs `script` and returns the error it raised, or `None` when it succeeds.
        fn raised_error(plugin: &mut PluginTest, script: &str) -> Option<ShellError> {
            plugin.eval(script).err()
        }

        #[rstest]
        fn opens_the_repo_flag_path(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let mut plugin = plugin_test?;

            // `gix open` reads `$env.PWD` even when `--repo` is absolute, so it must be set.
            pin_pwd(&mut plugin, repo.path());

            let script = format!("{OPEN_FLAG} {:?} | {DESCRIBE_COMMAND}", repo.path());
            let value = plugin.eval(&script)?.into_value(Span::test_data())?;

            assert_eq!(value.as_str()?, REPOSITORY_TYPE_NAME);

            Ok(())
        }

        #[rstest]
        fn opens_the_repository_containing_the_pinned_cwd(
            plugin_test: Result<PluginTest, FixtureError>,
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let mut plugin = plugin_test?;

            pin_pwd(&mut plugin, repo.path());

            let script = format!("{OPEN_COMMAND} | {DESCRIBE_COMMAND}");
            let value = plugin.eval(&script)?.into_value(Span::test_data())?;

            assert_eq!(value.as_str()?, REPOSITORY_TYPE_NAME);

            Ok(())
        }

        #[rstest]
        fn reports_not_a_repository_message(
            plugin_test: Result<PluginTest, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = plain_dir?;
            let missing = dir.path().join(MISSING_DIR);

            pin_pwd(&mut plugin, dir.path());

            let err = raised_error(&mut plugin, &format!("{OPEN_FLAG} {missing:?}"))
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_eq!(
                err.to_string(),
                format!("'{}' is not inside a git repository", missing.display())
            );

            Ok(())
        }

        #[rstest]
        fn reports_not_a_repository_code(
            plugin_test: Result<PluginTest, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = plain_dir?;
            let missing = dir.path().join(MISSING_DIR);

            pin_pwd(&mut plugin, dir.path());

            let err = raised_error(&mut plugin, &format!("{OPEN_FLAG} {missing:?}"))
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;
            let code = err.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(NOT_A_REPOSITORY_CODE));

            Ok(())
        }

        #[rstest]
        fn reports_not_a_repository_code_without_flag(
            plugin_test: Result<PluginTest, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = plain_dir?;

            pin_pwd(&mut plugin, dir.path());

            let err = raised_error(&mut plugin, OPEN_COMMAND)
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;
            let code = err.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(NOT_A_REPOSITORY_CODE));

            Ok(())
        }

        #[rstest]
        fn labels_outside_repository_without_flag(
            plugin_test: Result<PluginTest, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = plain_dir?;

            pin_pwd(&mut plugin, dir.path());

            let err = raised_error(&mut plugin, OPEN_COMMAND)
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;
            let ShellError::LabeledError(labeled) = &err else {
                return Err(TestError::Failure(format!(
                    "expected a labeled error, got {err:?}"
                )));
            };

            assert_eq!(
                labeled.labels.first().map(|label| label.text.as_str()),
                Some(DISCOVERY_LABEL)
            );

            Ok(())
        }

        #[rstest]
        fn helps_with_searched_path_without_flag(
            plugin_test: Result<PluginTest, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = plain_dir?;

            pin_pwd(&mut plugin, dir.path());

            let err = raised_error(&mut plugin, OPEN_COMMAND)
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            let ShellError::LabeledError(labeled) = &err else {
                return Err(TestError::Failure(format!(
                    "expected a labeled error, got {err:?}"
                )));
            };

            let help = labeled
                .help
                .as_deref()
                .ok_or_else(|| TestError::Failure("expected help text".to_owned()))?;

            let searched = dir.path().to_string_lossy();

            assert!(
                help.contains(searched.as_ref()),
                "help should name the searched directory, got {help:?}"
            );

            Ok(())
        }

        #[rstest]
        fn errors_with_open_failed_code_for_a_corrupt_repository(
            plugin_test: Result<PluginTest, FixtureError>,
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let mut plugin = plugin_test?;
            let dir = corrupt_repo?.dir;

            pin_pwd(&mut plugin, dir.path());

            let script = format!("{OPEN_FLAG} {:?}", dir.path());

            let err = plugin
                .eval(&script)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            let code = err.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(OPEN_FAILED_CODE));

            Ok(())
        }
    }

    mod run_open {
        use super::*;

        #[rstest]
        fn serves_later_acquires_from_the_cache(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let registry = Registry::new();
            let handle = run_open(&registry, repo.path(), None)?;
            let location = handle.location();

            // A rebuild would reopen the repository, which fails once its HEAD file is gone.
            fs::remove_file(location.git_dir.join(HEAD_FILE))?;

            assert!(
                Registry::new().acquire(handle.id, &location).is_err(),
                "a fresh registry must not be able to rebuild the repository"
            );

            let first = registry.acquire(handle.id, &location)?;
            let second = registry.acquire(handle.id, &location)?;

            assert!(Arc::ptr_eq(&first, &second));

            Ok(())
        }

        #[rstest]
        fn reports_unopenable_repository_as_git_error(
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = corrupt_repo?.dir;

            let err = run_open(&Registry::new(), dir.path(), None)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(err, CommandError::Git(GixError::Open { .. }));

            Ok(())
        }

        #[rstest]
        fn resolves_relative_repo_flag_against_cwd(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = plain_dir?;
            let nested = dir.path().join(NESTED_DIR);

            gix::init(&nested)?;

            let handle = run_open(
                &Registry::new(),
                dir.path(),
                Some(flag(Path::new(NESTED_DIR))),
            )?;

            assert_eq!(handle.path.canonicalize()?, nested.canonicalize()?);

            Ok(())
        }

        #[rstest]
        fn reports_missing_cwd_as_not_found(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = plain_dir?;

            let err = run_open(&Registry::new(), &dir.path().join(MISSING_DIR), None)
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(err, CommandError::Git(GixError::NotFound { .. }));

            Ok(())
        }

        #[rstest]
        fn discovers_repository_from_subdirectory(
            test_repo: Result<TestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let repo = test_repo?;
            let subdir = repo.path().join(SUBDIR);

            fs::create_dir_all(&subdir)?;

            let handle = run_open(&Registry::new(), &subdir, None)?;

            assert_eq!(
                handle.git_dir.canonicalize()?,
                repo.path().join(GIT_DIR).canonicalize()?
            );
            assert_eq!(
                handle
                    .worktree
                    .as_deref()
                    .map(Path::canonicalize)
                    .transpose()?,
                Some(repo.path().canonicalize()?)
            );

            Ok(())
        }

        #[rstest]
        fn opens_bare_repository_without_worktree(
            bare_repo: Result<UnbornTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = bare_repo?.dir;

            let handle = run_open(&Registry::new(), dir.path(), None)?;

            assert_eq!(handle.worktree, None);

            Ok(())
        }

        #[rstest]
        fn reports_flag_span_for_unopenable_repository(
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = corrupt_repo?.dir;

            let err = run_open(&Registry::new(), dir.path(), Some(flag(dir.path())))
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(&err, CommandError::OpenFailed { span, .. } if *span == FLAG_SPAN);

            Ok(())
        }

        #[rstest]
        fn reports_flag_span_for_non_repository(
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let dir = plain_dir?;

            let err = run_open(
                &Registry::new(),
                dir.path(),
                Some(flag(Path::new(MISSING_DIR))),
            )
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NotARepository { span, .. } if *span == FLAG_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }
}
