//! `gix open`: open a git repository, by default the one containing the current directory.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{CATEGORY, CommandError};

use nu_plugin::{EngineInterface, EvaluatedCall, SimplePluginCommand};
use nu_plugin_gix_core::{GixError, Registry};
use nu_protocol::{Category, Example, LabeledError, Signature, Spanned, SyntaxShape, Type, Value};

use std::path::{Path, PathBuf};

const COMMAND_NAME: &str = "gix open";
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
                .map_err(|source| match &source {
                    GixError::NotFound {
                        path: _path,
                        source: _source,
                    } => CommandError::not_a_repository(item, span, source),
                    GixError::Open {
                        path: _path,
                        source: _source,
                    } => CommandError::open_failed(item, span, source),
                    _ => CommandError::Git(source),
                })?,
        };

    Ok(Repository::new(id, location))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::plugin_test;

    use nu_plugin_gix_testing_support::{
        AssertionError, CorruptTestRepo, FixtureError, FixtureRepo as _, TestRepo,
        assert_label_in_script, corrupt_repo, eval_labeled_error, eval_value, pin_pwd, plain_dir,
        test_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use nu_protocol::{PipelineData, ShellError, Span};
    use rstest::rstest;
    use tempfile::TempDir;

    use std::assert_matches;

    const OPEN_COMMAND: &str = "gix open";
    const OPEN_FLAG: &str = "gix open --repo";
    const DESCRIBE_COMMAND: &str = "describe";
    const REPOSITORY_TYPE_NAME: &str = <Repository as Handle>::TYPE_NAME;
    const NOT_A_REPOSITORY_CODE: &str = "gix::not-a-repository";
    const OPEN_FAILED_CODE: &str = "gix::open-failed";
    const EXPECTED_ERROR: &str = "expected an error";
    const FLAG_SPAN: Span = Span { start: 5, end: 9 };
    // Missing, so neither the path nor any of its parents holds a repository.
    const MISSING_DIR: &str = "missing";

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] Box<ShellError>),

        #[error(transparent)]
        Assertion(#[from] AssertionError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    impl From<ShellError> for TestError {
        fn from(error: ShellError) -> Self {
            Self::Shell(Box::new(error))
        }
    }

    mod signature {
        use super::*;

        #[rstest]
        fn declares_the_name_repo_flag_and_repository_output() {
            let signature = Open.signature();

            assert_eq!(signature.name, COMMAND_NAME);
            assert!(
                signature.get_long_flag(REPO_FLAG).is_some(),
                "the `--repo` flag is missing"
            );
            assert!(
                signature
                    .input_output_types
                    .contains(&(Type::Nothing, Type::Custom(REPOSITORY_TYPE_NAME.into()),)),
                "got {:?}",
                signature.input_output_types
            );
        }
    }

    mod eval {
        use super::*;

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
            let value = eval_value(&mut plugin, &script, PipelineData::empty())?;

            assert_eq!(value.as_str()?, REPOSITORY_TYPE_NAME);

            Ok(())
        }

        mod errors {
            use super::*;

            /// How a script names the directory it opens: through `--repo`, or the current directory.
            #[derive(Clone, Copy, Debug)]
            enum Invocation {
                RepoFlag,
                CurrentDir,
            }

            #[rstest]
            #[case::repo_flag(Invocation::RepoFlag)]
            #[case::current_dir(Invocation::CurrentDir)]
            fn reports_a_non_repository_with_its_code_message_and_label(
                #[case] invocation: Invocation,
                plugin_test: Result<PluginTest, FixtureError>,
                plain_dir: Result<TempDir, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let dir = plain_dir?;
                let missing = dir.path().join(MISSING_DIR);

                pin_pwd(&mut plugin, dir.path());

                let (script, searched) = match invocation {
                    Invocation::RepoFlag => (format!("{OPEN_FLAG} {missing:?}"), missing.as_path()),
                    Invocation::CurrentDir => (OPEN_COMMAND.to_owned(), dir.path()),
                };

                let labeled = eval_labeled_error(&mut plugin, &script, PipelineData::empty())?;
                let searched = searched.to_string_lossy();

                assert_eq!(labeled.code.as_deref(), Some(NOT_A_REPOSITORY_CODE));
                assert!(
                    labeled.msg.contains(searched.as_ref()),
                    "message should name {searched}, got {:?}",
                    labeled.msg
                );
                assert_label_in_script!(&labeled, &script);

                Ok(())
            }

            #[rstest]
            fn reports_an_unopenable_repository_with_its_code_message_and_label(
                plugin_test: Result<PluginTest, FixtureError>,
                corrupt_repo: Result<CorruptTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let dir = corrupt_repo?.dir;

                pin_pwd(&mut plugin, dir.path());

                let script = format!("{OPEN_FLAG} {:?}", dir.path());
                let labeled = eval_labeled_error(&mut plugin, &script, PipelineData::empty())?;
                let path = dir.path().to_string_lossy();

                assert_eq!(labeled.code.as_deref(), Some(OPEN_FAILED_CODE));
                assert!(
                    labeled.msg.contains(path.as_ref()),
                    "message should name {path}, got {:?}",
                    labeled.msg
                );
                assert_label_in_script!(&labeled, &script);

                Ok(())
            }
        }
    }

    mod run_open {
        use super::*;

        /// The `--repo` target a case builds, which also fixes the error it should raise.
        #[derive(Clone, Copy, Debug)]
        enum FlagTarget {
            UnopenableRepository,
            NonRepository,
        }

        #[rstest]
        #[case::unopenable_repository(FlagTarget::UnopenableRepository)]
        #[case::non_repository(FlagTarget::NonRepository)]
        fn points_a_flag_failure_at_the_flag(
            #[case] target: FlagTarget,
            corrupt_repo: Result<CorruptTestRepo, FixtureError>,
            plain_dir: Result<TempDir, FixtureError>,
        ) -> Result<(), TestError> {
            let (dir, flag_path) = match target {
                FlagTarget::UnopenableRepository => {
                    let dir = corrupt_repo?.dir;
                    let flag_path = dir.path().to_owned();

                    (dir, flag_path)
                }
                FlagTarget::NonRepository => (plain_dir?, PathBuf::from(MISSING_DIR)),
            };

            let flag = Spanned {
                item: flag_path,
                span: FLAG_SPAN,
            };
            let err = run_open(&Registry::new(), dir.path(), Some(flag))
                .err()
                .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            match target {
                FlagTarget::UnopenableRepository => assert_matches!(
                    &err,
                    CommandError::OpenFailed { span, .. } if *span == FLAG_SPAN,
                    "got {err:?}"
                ),
                FlagTarget::NonRepository => assert_matches!(
                    &err,
                    CommandError::NotARepository { span, .. } if *span == FLAG_SPAN,
                    "got {err:?}"
                ),
            }

            Ok(())
        }
    }
}
