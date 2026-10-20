//! `gix log`: stream the commits of an opened repository, selected by revision, references, paths and window, as a Nushell `ListStream`.

use crate::GixPlugin;
use crate::cdt::{Handle, Repository};
use crate::commands::{
    CATEGORY, CommandError, acquire_handle, argument_span, flag_arg, handle_from, input_value,
    non_blank_revision, switch_arg,
};

use nu_plugin::{EngineInterface, EvaluatedCall, PluginCommand};
use nu_plugin_gix_core::{
    self as git, CommitInfo, FileChange, FileChangeKind, FileStat, GixError, LogSettings, LogStart,
    Mailmap, Pattern, Registry, SignatureInfo,
};
use nu_protocol::{
    Category, Example, LabeledError, ListStream, PipelineData, Record, ShellError, Signals,
    Signature, Span, Spanned, SyntaxShape, Type, Value, record,
};

use std::path::Path;

const COMMAND_NAME: &str = "gix log";
const REVISIONS_ARG: &str = "revisions";
const REVISIONS_POSITION: usize = 0;
const SKIP_FLAG: &str = "skip";
const SKIP_SHORT: char = 's';
const LIMIT_FLAG: &str = "limit";
const LIMIT_SHORT: char = 'n';
const ALL_FLAG: &str = "all";
const ALL_SHORT: char = 'a';
const FIRST_PARENT_FLAG: &str = "first-parent";
const NO_MAILMAP_FLAG: &str = "no-mailmap";
const REVERSE_FLAG: &str = "reverse";
const REVERSE_SHORT: char = 'r';
const PATHS_FLAG: &str = "paths";
const PATHS_SHORT: char = 'p';
const OID_COLUMN: &str = "oid";
const PARENTS_COLUMN: &str = "parents";
const TREE_COLUMN: &str = "tree";
/// Holds the title paragraph on one line.
const SUBJECT_COLUMN: &str = "subject";
/// Holds the rest of the message, or null when there is none.
const BODY_COLUMN: &str = "body";
/// Holds the message exactly as stored.
const MESSAGE_COLUMN: &str = "message";
const AUTHOR_COLUMN: &str = "author";
const COMMITTER_COLUMN: &str = "committer";
const NAME_FIELD: &str = "name";
const EMAIL_FIELD: &str = "email";
const TIME_FIELD: &str = "time";
const FILES_FLAG: &str = "files";
const FILES_COLUMN: &str = "files";
const STAT_FLAG: &str = "stat";
const INSERTIONS_FIELD: &str = "insertions";
const DELETIONS_FIELD: &str = "deletions";
const PATH_FIELD: &str = "path";
const CHANGE_FIELD: &str = "change";
const ADDED: &str = "added";
const DELETED: &str = "deleted";
const MODIFIED: &str = "modified";

/// Streams the commits a revision, range or `--all` selects from a `git repository` value, so large histories are never fully buffered.
#[derive(Clone, Copy, Debug)]
pub struct Log;

impl PluginCommand for Log {
    type Plugin = GixPlugin;

    fn name(&self) -> &'static str {
        COMMAND_NAME
    }

    fn description(&self) -> &'static str {
        "List commits of a repository opened with `gix open`, newest first by commit time: from HEAD, a revision or range, or every reference."
    }

    fn extra_description(&self) -> &'static str {
        "Columns: oid, parents, tree, subject (the title paragraph on one line), body (the rest, trailing whitespace trimmed; null when there is none), message (as stored), and author/committer records of name, email and time.\n\
        --files adds a `files` column: a list of `{path, change}` records for the files each commit changes against its first parent (the root commit against an empty tree), with change `added`, `deleted` or `modified`. Renames are not detected, so a move shows as a deletion and an addition, and --paths limits the list to matching paths.\n\
        --stat implies --files and adds `insertions` and `deletions` to each file: the lines added and removed against the first parent, both null for a binary file or one whose blob could not be read. Renames are not tracked, so a moved file counts in full.\n\
        Author and committer are mapped through the repository's mailmap (.mailmap, mailmap.file, mailmap.blob) as `git log` does; --no-mailmap shows them as recorded.\n\
        Several revisions may be given; their includes and hides are united, and --all adds every reference (e.g. `--all '^main'`). Without a revision or --all the walk starts at HEAD. --all skips references that cannot be read and lists nothing in an empty repository.\n\
        --paths are git pathspecs: globs (`*.rs`), `:(exclude)`/`:!`, `:(icase)`, `:(top)`/`:/`, `:(literal)` and `:(glob)`. They are relative to the current directory unless they use top magic or are absolute, and must lie inside the repository's worktree; in a bare repository they are relative to the repository root. Attribute pathspecs such as `:(attr:...)` are not supported. A directory matches every change below it, and `.` matches any change below the current directory. A merge identical to a parent at those paths is not listed, and only the history of the first such parent is followed, as `git log -- <paths>` does. With --first-parent only the first parent is compared, as `git log --first-parent -- <paths>` does.\n\
        --skip and --limit count commits after path filtering; --reverse applies after both and buffers the selection.\n\
        Error codes: gix::wrong-input, gix::invalid-input, gix::invalid-argument, gix::current-dir-unavailable, gix::invalid-revision, gix::negative-limit, gix::negative-skip, gix::path-outside-repository, gix::invalid-pathspec, gix::open-failed, gix::repository-gone, gix::bad-revision, gix::not-a-commit, gix::unborn-head, gix::ref-read-failed, gix::walk-failed, gix::commit-unreadable, gix::bad-timestamp, gix::io, gix::walker-panicked"
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "gix open | gix log",
                description: "List the commits reachable from HEAD, newest first",
                result: None,
            },
            Example {
                example: "gix open | gix log --limit 10",
                description: "List the ten most recent commits",
                result: None,
            },
            Example {
                example: "gix open | gix log --no-mailmap --limit 5",
                description: "List the five most recent commits with authors and committers as recorded",
                result: None,
            },
            Example {
                example: "gix open | gix log main..feature",
                description: "List the commits on `feature` that are not on `main`",
                result: None,
            },
            Example {
                example: "gix open | gix log --paths [src/main.rs] --first-parent",
                description: "From the repository root, list the first-parent commits that changed `src/main.rs`",
                result: None,
            },
            Example {
                example: "gix open | gix log --paths ['*.rs' ':(exclude)tests']",
                description: "List the commits that change a Rust file outside `tests`",
                result: None,
            },
            Example {
                example: "gix open | gix log --all --reverse --skip 5",
                description: "List every branch's history oldest first, without the five newest commits",
                result: None,
            },
            Example {
                example: "gix open | gix log --all '^main'",
                description: "List every reference's history except the commits already on `main`",
                result: None,
            },
            Example {
                example: "gix open | gix log --limit 1 | get 0.message",
                description: "Read the full message of the newest commit, like `git log -1 --format=%B`",
                result: None,
            },
            Example {
                example: "gix open | gix log --files --limit 1 | get 0.files",
                description: "List the files the newest commit changes, each with its `change`",
                result: None,
            },
            Example {
                example: "gix open | gix log --stat --limit 1 | get 0.files",
                description: "List the files the newest commit changes, each with its `insertions` and `deletions`",
                result: None,
            },
        ]
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .rest(
                REVISIONS_ARG,
                SyntaxShape::String,
                "revisions or ranges to list, e.g. `main`, `v1..v2`, `a...b`, `^x`; defaults to HEAD unless --all",
            )
            .named(
                PATHS_FLAG,
                SyntaxShape::List(Box::new(SyntaxShape::String)),
                "only commits that change one of these paths, relative to the current directory",
                Some(PATHS_SHORT),
            )
            .named(
                SKIP_FLAG,
                SyntaxShape::Int,
                "number of selected commits to leave out, newest first",
                Some(SKIP_SHORT),
            )
            .named(
                LIMIT_FLAG,
                SyntaxShape::Int,
                "maximum number of commits to return",
                Some(LIMIT_SHORT),
            )
            .switch(
                ALL_FLAG,
                "add every reference and HEAD to the revisions given, as `git log --all` does",
                Some(ALL_SHORT),
            )
            .switch(
                FIRST_PARENT_FLAG,
                "follow only the first parent of merge commits",
                None,
            )
            .switch(
                REVERSE_FLAG,
                "list the selected commits oldest first (after --skip and --limit)",
                Some(REVERSE_SHORT),
            )
            .switch(
                NO_MAILMAP_FLAG,
                "show author and committer as recorded, without the repository's mailmap",
                None,
            )
            .switch(
                FILES_FLAG,
                "add a `files` column listing the files each commit changes against its first parent",
                None,
            )
            .switch(
                STAT_FLAG,
                "add `insertions` and `deletions` to each file, the lines added and removed against the first parent (null for binary or unreadable files); implies --files",
                None,
            )
            .input_output_type(
                Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                Type::table(),
            )
            .category(Category::Custom(CATEGORY.into()))
    }

    fn run(
        &self,
        plugin: &GixPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        log_call(plugin.registry(), engine, call, input).map_err(|e| e.into_labeled(call.head))
    }
}

/// Reads the pipeline input, the selection flags and the engine signals from `call`, then streams the commits they select.
///
/// The registry supplies the opened repository, rebuilding it from the handle's location when it is not cached.
/// A handle whose repository is gone or cannot be reopened is reported at the input's span, the value that carried it.
/// A failure to start the walk from a revision is reported at that revision, see [`log_failure`].
fn log_call(
    registry: &Registry,
    engine: &EngineInterface,
    call: &EvaluatedCall,
    input: PipelineData,
) -> Result<PipelineData, CommandError> {
    let span = call.head;
    let input = input_value(input, span)?;
    let repo = handle_from::<Repository>(&input)?;

    // A skip beyond `usize::MAX` skips every commit; a limit beyond it is no limit.
    let skip = flag_arg::<i64>(call, SKIP_FLAG)?
        .map(|skip| {
            non_negative(skip, |skip, skip_span| CommandError::NegativeSkip {
                skip,
                span: skip_span,
            })
        })
        .transpose()?
        .unwrap_or_default();

    let limit = flag_arg::<i64>(call, LIMIT_FLAG)?
        .map(|limit| {
            non_negative(limit, |limit, limit_span| CommandError::NegativeLimit {
                limit,
                span: limit_span,
            })
        })
        .transpose()?;

    let revisions = call
        .rest::<Spanned<String>>(REVISIONS_POSITION)
        .map_err(|source| CommandError::InvalidArgument {
            source: Box::new(source),
            span: argument_span(call, REVISIONS_POSITION),
        })?
        .into_iter()
        .map(non_blank_revision)
        .collect::<Result<Vec<_>, _>>()?;

    let paths = flag_arg::<Vec<Spanned<String>>>(call, PATHS_FLAG)?
        .map_or_else(Vec::new, |paths| paths.item);

    let paths = if paths.is_empty() {
        Vec::new()
    } else {
        let cwd = engine
            .get_current_dir()
            .map_err(|source| CommandError::CurrentDir {
                source: Box::new(source),
                span,
            })?;

        resolve_paths(repo.worktree.as_deref(), Path::new(&cwd), paths)?
    };

    let start = LogStart {
        revisions: revisions
            .iter()
            .map(|revision| revision.item.clone())
            .collect(),
        all: switch_arg(call, ALL_FLAG)?,
    };

    let mailmap = if switch_arg(call, NO_MAILMAP_FLAG)? {
        Mailmap::Ignore
    } else {
        Mailmap::Apply
    };

    let settings = LogSettings {
        first_parent: switch_arg(call, FIRST_PARENT_FLAG)?,
        reverse: switch_arg(call, REVERSE_FLAG)?,
        skip,
        limit,
        mailmap,
        files: switch_arg(call, FILES_FLAG)?,
        stat: switch_arg(call, STAT_FLAG)?,
    };

    let opened = acquire_handle(registry, repo, input.span())?;
    let signals = engine.signals().clone();

    let commits = git::log(opened, &start, &paths, settings)
        .map_err(|source| log_failure(source, &revisions, span))?
        .with_interrupt({
            let signals = signals.clone();
            move || signals.interrupted()
        });

    Ok(commit_stream(commits, span, signals).into())
}

/// Resolves each `--paths` entry, a git pathspec written relative to `cwd` or absolute, against the
/// repository root; an entry that is not a valid pathspec, or lies outside the repository, is
/// reported at its own span.
fn resolve_paths(
    worktree: Option<&Path>,
    cwd: &Path,
    paths: Vec<Spanned<String>>,
) -> Result<Vec<Pattern>, CommandError> {
    paths
        .into_iter()
        .map(|value| {
            git::pathspec(worktree, cwd, &value.item).map_err(|source| CommandError::Located {
                span: value.span,
                source,
            })
        })
        .collect()
}

/// Maps each streamed commit to a row spanning `span`, and wraps the rows in a stream that the engine's `signals` end.
///
/// The walk runs in the background; it stops when the stream is dropped or the engine signals an interrupt (Ctrl-C),
/// which the caller arms on `commits` with `CommitLog::with_interrupt`.
fn commit_stream<I>(commits: I, span: Span, signals: Signals) -> ListStream
where
    I: Iterator<Item = Result<CommitInfo, GixError>> + Send + 'static,
{
    let values = commits.map(move |commit| commit_value(commit, span));

    ListStream::new(values, span, signals)
}

/// Attributes a failure to start the walk: a revision that does not resolve or names no commit to
/// the span of the revision it names among `revisions`, anything else to `call_span`. A revision
/// the git layer names but the call did not give falls back to `call_span`.
fn log_failure(source: GixError, revisions: &[Spanned<String>], call_span: Span) -> CommandError {
    let expression = match &source {
        GixError::Revision {
            expression,
            source: _source,
        } => Some(expression),
        GixError::NotACommit {
            expression,
            kind: _kind,
        } => Some(expression),
        _ => None,
    };

    let span = expression.map(|expression| {
        revisions
            .iter()
            .find(|revision| revision.item == *expression)
            .map_or(call_span, |revision| revision.span)
    });

    match span {
        Some(span) => CommandError::Located { span, source },
        None => CommandError::Git(source),
    }
}

/// Converts a requested count; a negative value is rejected with the error `negative` builds from the value
/// and its span, and a value beyond `usize::MAX` saturates to it.
fn non_negative<F>(value: Spanned<i64>, negative: F) -> Result<usize, CommandError>
where
    F: FnOnce(i64, Span) -> CommandError,
{
    if value.item.is_negative() {
        Err(negative(value.item, value.span))
    } else {
        Ok(usize::try_from(value.item).unwrap_or(usize::MAX))
    }
}

/// Maps one streamed commit to a table row, or to an error value that ends the stream.
fn commit_value(commit: Result<CommitInfo, GixError>, span: Span) -> Value {
    match commit {
        Ok(commit) => commit_record(commit, span),
        Err(err) => Value::error(
            ShellError::LabeledError(Box::new(CommandError::from(err).into_labeled(span))),
            span,
        ),
    }
}

/// Maps a commit to a row; `body` is null when the message has none, and `author` and `committer`
/// become `{ name, email, time }` records. `files`, when the commit carries them, becomes a list of
/// `{ path, change }` records.
fn commit_record(commit: CommitInfo, span: Span) -> Value {
    let parents = commit
        .parents
        .into_iter()
        .map(|parent| Value::string(parent.to_string(), span))
        .collect();

    let body = commit
        .body
        .map_or_else(|| Value::nothing(span), |body| Value::string(body, span));

    let mut record: Record = record! {
        OID_COLUMN => Value::string(commit.oid.to_string(), span),
        PARENTS_COLUMN => Value::list(parents, span),
        TREE_COLUMN => Value::string(commit.tree.to_string(), span),
        SUBJECT_COLUMN => Value::string(commit.subject, span),
        BODY_COLUMN => body,
        MESSAGE_COLUMN => Value::string(commit.message, span),
        AUTHOR_COLUMN => signature_record(commit.author, span),
        COMMITTER_COLUMN => signature_record(commit.committer, span),
    };

    if let Some(files) = commit.files {
        record.push(FILES_COLUMN, files_value(files, span));
    }

    Value::record(record, span)
}

/// Maps a signature to a `{ name, email, time }` record with `time` a date in the signer's offset.
fn signature_record(signature: SignatureInfo, span: Span) -> Value {
    let record: Record = record! {
        NAME_FIELD => Value::string(signature.name, span),
        EMAIL_FIELD => Value::string(signature.email, span),
        TIME_FIELD => Value::date(signature.time, span),
    };

    Value::record(record, span)
}

/// Maps the files a commit changes to a list of `{ path, change }` records, adding `insertions` and
/// `deletions` to each file that carries a stat; a binary or unavailable file has null counts.
fn files_value(files: Vec<FileChange>, span: Span) -> Value {
    let files = files
        .into_iter()
        .map(|FileChange { path, kind, stat }| {
            let mut record: Record = record! {
                PATH_FIELD => Value::string(path, span),
                CHANGE_FIELD => Value::string(change_name(kind), span),
            };

            if let Some(stat) = stat {
                let (insertions, deletions) = match stat {
                    FileStat::Text {
                        insertions,
                        deletions,
                    } => (
                        Value::int(i64::from(insertions), span),
                        Value::int(i64::from(deletions), span),
                    ),
                    FileStat::Binary | FileStat::Unavailable => {
                        (Value::nothing(span), Value::nothing(span))
                    }
                };

                record.push(INSERTIONS_FIELD, insertions);
                record.push(DELETIONS_FIELD, deletions);
            }

            Value::record(record, span)
        })
        .collect();

    Value::list(files, span)
}

/// The name a file change is listed under in the `change` field.
const fn change_name(kind: FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Addition => ADDED,
        FileChangeKind::Deletion => DELETED,
        FileChangeKind::Modification => MODIFIED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures::{RepoInputError, plugin_test, repo_input};

    use miette::Diagnostic as _;
    use nu_plugin_gix_testing_support::{
        AssertionError, COMMITTER_NAME, FixtureError, FixtureRepo as _, GIT_DIR, HEAD_FILE,
        MAIN_BRANCH, PATHS_DIR, PATHS_FILE, PATHS_NESTED_FILE, PATHS_NESTED_NAME, PathsTestRepo,
        TestRepo, UnbornTestRepo, assert_label_in_script, diverged_repo, eval_labeled_error,
        eval_value, first_row, forked_repo, gix_error, mailmap_repo, paths_repo, pin_pwd,
        record_column, test_repo, unborn_repo,
    };
    use nu_plugin_test_support::PluginTest;
    use rstest::rstest;
    use tempfile::TempDir;

    use std::{assert_matches, fs, io, path::Path};

    const EXPECTED_ERROR: &str = "expected an error";
    const PATHS_ROW_COUNT: usize = 2;
    const PATH_OUTSIDE_REPOSITORY_CODE: &str = "gix::path-outside-repository";
    const PATH_OUTSIDE_REPOSITORY_SCRIPT: &str = "gix log --paths [../outside]";
    const INVALID_PATHSPEC_CODE: &str = "gix::invalid-pathspec";
    const INVALID_PATHSPEC_SCRIPT: &str = "gix log --paths [':(nonsense)x']";
    const PATHS_GLOB: &str = "'*.txt'";
    const PATHS_GLOB_ROW_COUNT: usize = 4;
    const NEGATIVE_LIMIT: i64 = -1;
    const NEGATIVE_LIMIT_CODE: &str = "gix::negative-limit";
    const NEGATIVE_LIMIT_SCRIPT: &str = "gix log --limit -1";
    const NEGATIVE_SKIP: i64 = -1;
    const NEGATIVE_SKIP_CODE: &str = "gix::negative-skip";
    const NEGATIVE_SKIP_SCRIPT: &str = "gix log --skip -1";
    const SKIP_VALUE_SPAN: Span = Span { start: 10, end: 12 };
    const ONE: usize = 1;
    const LIMITED_ROW_COUNT: usize = 1;
    const UNBORN_HEAD_CODE: &str = "gix::unborn-head";
    const HANDLE_REPOSITORY_GONE_CODE: &str = "gix::repository-gone";
    const HANDLE_OPEN_FAILED_CODE: &str = "gix::open-failed";
    const BAD_TIMESTAMP_CODE: &str = "gix::bad-timestamp";
    const BAD_TIMESTAMP_ID: &str = "abc";
    const DIVERGED_ROW_COUNT: usize = 3;
    const FORKED_FIRST_PARENT_ROW_COUNT: usize = 3;
    const LIMIT_VALUE_SPAN: Span = Span { start: 10, end: 12 };
    const ERROR_SPAN: Span = Span { start: 40, end: 47 };
    const CALL_SPAN: Span = Span { start: 0, end: 7 };
    const REVISION_SPAN: Span = Span { start: 8, end: 14 };
    const PARENT_REVISION: &str = "HEAD~1";
    const UNKNOWN_REVISION: &str = "not-a-revision";
    const TREE_REVISION: &str = "HEAD^{tree}";
    const TREE_KIND: &str = "tree";
    const BLANK_REVISION_SCRIPT: &str = "gix log ''";
    const INVALID_REVISION_CODE: &str = "gix::invalid-revision";
    const UNKNOWN_REVISION_SCRIPT: &str = "gix log not-a-revision";
    const BAD_REVISION_CODE: &str = "gix::bad-revision";
    const NOT_A_COMMIT_SCRIPT: &str = "gix log 'HEAD^{tree}'";
    const NOT_A_COMMIT_CODE: &str = "gix::not-a-commit";
    const HEAD_REVISION: &str = "HEAD";
    const EXCLUDED_PARENT: &str = "^HEAD~1";
    const EXCLUDED_BRANCH: &str = "^main";
    const SECOND_REVISION_SPAN: Span = Span { start: 15, end: 22 };
    const ROW_COLUMNS: [&str; 8] = [
        OID_COLUMN,
        PARENTS_COLUMN,
        TREE_COLUMN,
        SUBJECT_COLUMN,
        BODY_COLUMN,
        MESSAGE_COLUMN,
        AUTHOR_COLUMN,
        COMMITTER_COLUMN,
    ];
    const SAMPLE_OID: &str = "1111111111111111111111111111111111111111";
    const SAMPLE_PARENTS: [&str; 2] = [
        "2222222222222222222222222222222222222222",
        "3333333333333333333333333333333333333333",
    ];
    const SAMPLE_TREE: &str = "4444444444444444444444444444444444444444";
    const SAMPLE_SUBJECT: &str = "Subject line";
    const SAMPLE_BODY: &str = "First body line\n\nSecond body line";
    const SAMPLE_MESSAGE: &str = "Subject line\n\nFirst body line\n\nSecond body line\n";
    const SAMPLE_AUTHOR_NAME: &str = "Ada Author";
    const SAMPLE_AUTHOR_EMAIL: &str = "ada@example.com";
    const SAMPLE_COMMITTER_NAME: &str = "Grace Committer";
    const SAMPLE_COMMITTER_EMAIL: &str = "grace@example.com";
    const SAMPLE_TIME: &str = "2024-03-01T10:20:30+02:00";
    const STAT_INSERTIONS: u32 = 2;
    const STAT_DELETIONS: u32 = 1;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Shell(#[from] Box<ShellError>),

        #[error(transparent)]
        Assertion(#[from] AssertionError),

        #[error(transparent)]
        Fixture(#[from] FixtureError),

        #[error(transparent)]
        Input(#[from] RepoInputError),

        #[error(transparent)]
        Git(#[from] GixError),

        #[error(transparent)]
        Id(#[from] gix::Error),

        #[error(transparent)]
        Io(#[from] io::Error),

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
        fn declares_the_name_and_repository_input() {
            let signature = Log.signature();

            assert_eq!(signature.name, COMMAND_NAME);

            assert!(
                signature.input_output_types.contains(&(
                    Type::Custom(<Repository as Handle>::TYPE_NAME.into()),
                    Type::table(),
                )),
                "got {:?}",
                signature.input_output_types
            );
        }

        #[rstest]
        #[case::skip(SKIP_FLAG, Some(SKIP_SHORT))]
        #[case::limit(LIMIT_FLAG, Some(LIMIT_SHORT))]
        #[case::all(ALL_FLAG, Some(ALL_SHORT))]
        #[case::first_parent(FIRST_PARENT_FLAG, None)]
        #[case::reverse(REVERSE_FLAG, Some(REVERSE_SHORT))]
        #[case::paths(PATHS_FLAG, Some(PATHS_SHORT))]
        #[case::no_mailmap(NO_MAILMAP_FLAG, None)]
        #[case::files(FILES_FLAG, None)]
        #[case::stat(STAT_FLAG, None)]
        fn declares_each_flag_with_its_short_form(
            #[case] flag: &str,
            #[case] short: Option<char>,
        ) -> Result<(), TestError> {
            let signature = Log.signature();

            let declared = signature
                .get_long_flag(flag)
                .ok_or_else(|| TestError::Failure(format!("missing --{flag}")))?;

            assert_eq!(declared.short, short);

            Ok(())
        }

        #[rstest]
        fn declares_revisions_as_its_rest_positional() {
            let signature = Log.signature();

            assert_eq!(
                signature
                    .rest_positional
                    .as_ref()
                    .map(|arg| arg.name.as_str()),
                Some(REVISIONS_ARG)
            );
        }
    }

    mod log {
        use super::*;

        mod rows {
            use super::*;

            #[rstest]
            fn expose_the_documented_columns_with_dated_signatures(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let table = eval_value(&mut plugin, COMMAND_NAME, repo_input(repo.path())?)?;
                let row = first_row(&table)?;
                let record = row.as_record()?;
                let columns: Vec<&str> = record.columns().map(String::as_str).collect();
                let author = record_column(row, AUTHOR_COLUMN)?;
                let time = record_column(author, TIME_FIELD)?;

                assert_eq!(columns, ROW_COLUMNS);
                assert_matches!(time, Value::Date { .. });

                Ok(())
            }
        }

        mod first {
            use super::*;

            /// A consumer that takes one row must get a record with the documented columns.
            #[rstest]
            fn yields_a_single_row_with_the_documented_columns(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let row = eval_value(
                    &mut plugin,
                    &format!("{COMMAND_NAME} | first"),
                    repo_input(repo.path())?,
                )?;

                let columns: Vec<&str> = row.as_record()?.columns().map(String::as_str).collect();

                assert_eq!(columns, ROW_COLUMNS);

                Ok(())
            }
        }

        mod forwarding {
            use super::*;

            /// A flag or argument `gix log` forwards to the git layer, with the fixture that shows its effect.
            #[derive(Clone, Copy, Debug)]
            enum Forwarded {
                Skip,
                Limit,
                All,
                FirstParent,
                Revision,
                Revisions,
                AllWithExclusion,
                Paths,
            }

            impl Forwarded {
                fn script(self) -> String {
                    match self {
                        Self::Skip => format!("{COMMAND_NAME} --{SKIP_FLAG} {ONE}"),
                        Self::Limit => format!("{COMMAND_NAME} --{LIMIT_FLAG} {ONE}"),
                        Self::All => format!("{COMMAND_NAME} --{ALL_FLAG}"),
                        Self::FirstParent => format!("{COMMAND_NAME} --{FIRST_PARENT_FLAG}"),
                        Self::Revision => format!("{COMMAND_NAME} {PARENT_REVISION}"),
                        Self::Revisions => {
                            format!("{COMMAND_NAME} {HEAD_REVISION} '{EXCLUDED_PARENT}'")
                        }
                        Self::AllWithExclusion => {
                            format!("{COMMAND_NAME} --{ALL_FLAG} '{EXCLUDED_BRANCH}'")
                        }
                        Self::Paths => format!("{COMMAND_NAME} --{PATHS_FLAG} [{PATHS_GLOB}]"),
                    }
                }

                /// The fixture repository the script runs against; dropping it deletes the repository.
                fn repository(self) -> Result<TempDir, FixtureError> {
                    match self {
                        Self::Skip | Self::Limit | Self::Revision | Self::Revisions => {
                            Ok(test_repo()?.dir)
                        }
                        Self::All | Self::AllWithExclusion => Ok(diverged_repo()?.dir),
                        Self::FirstParent => Ok(forked_repo()?.dir),
                        Self::Paths => Ok(paths_repo()?.dir),
                    }
                }
            }

            #[rstest]
            #[case::skip(Forwarded::Skip, LIMITED_ROW_COUNT)]
            #[case::limit(Forwarded::Limit, LIMITED_ROW_COUNT)]
            #[case::all(Forwarded::All, DIVERGED_ROW_COUNT)]
            #[case::first_parent(Forwarded::FirstParent, FORKED_FIRST_PARENT_ROW_COUNT)]
            #[case::revision(Forwarded::Revision, ONE)]
            #[case::revisions(Forwarded::Revisions, LIMITED_ROW_COUNT)]
            #[case::all_with_exclusion(Forwarded::AllWithExclusion, LIMITED_ROW_COUNT)]
            #[case::paths(Forwarded::Paths, PATHS_GLOB_ROW_COUNT)]
            fn forwards_the_value_so_the_row_count_changes(
                plugin_test: Result<PluginTest, FixtureError>,
                #[case] forwarded: Forwarded,
                #[case] rows: usize,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let dir = forwarded.repository()?;

                // `--paths` resolves against `$env.PWD`, which the bare test engine lacks.
                pin_pwd(&mut plugin, dir.path());

                let table = eval_value(&mut plugin, &forwarded.script(), repo_input(dir.path())?)?;

                assert_eq!(table.as_list()?.len(), rows);

                Ok(())
            }
        }

        mod reverse {
            use super::*;

            /// The only observable effect of `--reverse`; the ordering itself is tested in core.
            #[rstest]
            fn lists_the_root_commit_first(
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                let table = eval_value(
                    &mut plugin,
                    &format!("{COMMAND_NAME} --{REVERSE_FLAG}"),
                    repo_input(repo.path())?,
                )?;

                let oid = record_column(first_row(&table)?, OID_COLUMN)?.as_str()?;

                assert_eq!(oid, repo.parent);

                Ok(())
            }
        }

        mod files {
            use super::*;

            /// `--files` forwards to the git layer: the newest commit of the fixture changes `dir/b.txt`, the second newest is read here.
            #[rstest]
            fn files_lists_the_changed_paths(
                plugin_test: Result<PluginTest, FixtureError>,
                paths_repo: Result<PathsTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = paths_repo?;

                let table = eval_value(
                    &mut plugin,
                    &format!(
                        "{COMMAND_NAME} --{FILES_FLAG} --{SKIP_FLAG} {ONE} --{LIMIT_FLAG} {ONE}"
                    ),
                    repo_input(repo.path())?,
                )?;

                let files = record_column(first_row(&table)?, FILES_COLUMN)?.as_list()?;

                let [file] = files else {
                    return Err(TestError::Failure(format!(
                        "expected one changed file, got {files:?}"
                    )));
                };

                assert_eq!(
                    record_column(file, PATH_FIELD)?.as_str()?,
                    PATHS_NESTED_FILE
                );

                Ok(())
            }

            /// `--stat` forwards to the git layer and implies `--files`: it alone lists the file that `files_lists_the_changed_paths`
            /// reads, now with its `insertions` and `deletions` columns.
            #[rstest]
            fn stat_lists_the_changed_paths_with_their_counts(
                plugin_test: Result<PluginTest, FixtureError>,
                paths_repo: Result<PathsTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = paths_repo?;

                let table = eval_value(
                    &mut plugin,
                    &format!(
                        "{COMMAND_NAME} --{STAT_FLAG} --{SKIP_FLAG} {ONE} --{LIMIT_FLAG} {ONE}"
                    ),
                    repo_input(repo.path())?,
                )?;

                let files = record_column(first_row(&table)?, FILES_COLUMN)?.as_list()?;

                let [file] = files else {
                    return Err(TestError::Failure(format!(
                        "expected one changed file, got {files:?}"
                    )));
                };

                assert_eq!(
                    record_column(file, PATH_FIELD)?.as_str()?,
                    PATHS_NESTED_FILE
                );

                assert_matches!(record_column(file, INSERTIONS_FIELD)?, Value::Int { .. });
                assert_matches!(record_column(file, DELETIONS_FIELD)?, Value::Int { .. });

                Ok(())
            }
        }

        mod mailmap {
            use super::*;

            /// `--no-mailmap` keeps the author as recorded; the fixture's `.mailmap` maps the committer name away.
            #[rstest]
            fn no_mailmap_shows_the_recorded_author(
                plugin_test: Result<PluginTest, FixtureError>,
                mailmap_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = mailmap_repo?;

                let table = eval_value(
                    &mut plugin,
                    &format!("{COMMAND_NAME} --{NO_MAILMAP_FLAG} --{LIMIT_FLAG} {ONE}"),
                    repo_input(repo.path())?,
                )?;

                let name = record_column(
                    record_column(first_row(&table)?, AUTHOR_COLUMN)?,
                    NAME_FIELD,
                )?;

                assert_eq!(name.as_str()?, COMMITTER_NAME);

                Ok(())
            }
        }

        mod paths {
            use super::*;

            /// Running from a subdirectory: `b.txt` means `dir/b.txt`; path semantics are tested in core.
            #[rstest]
            fn resolves_paths_against_the_current_directory(
                plugin_test: Result<PluginTest, FixtureError>,
                paths_repo: Result<PathsTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = paths_repo?;

                pin_pwd(&mut plugin, repo.path().join(PATHS_DIR));

                let table = eval_value(
                    &mut plugin,
                    &format!("{COMMAND_NAME} --{PATHS_FLAG} [{PATHS_NESTED_NAME}]"),
                    repo_input(repo.path())?,
                )?;

                assert_eq!(table.as_list()?.len(), PATHS_ROW_COUNT);

                Ok(())
            }
        }

        mod resolve_paths {
            use super::*;

            /// A worktree path that need not exist: `pathspec` resolves lexically.
            const WORKTREE: &str = "/work/repo";
            const OUTSIDE_PATH: &str = "../outside";
            const ABSOLUTE_OUTSIDE_PATH: &str = "/elsewhere/outside";
            const FIRST_PATH_SPAN: Span = Span { start: 15, end: 20 };
            const SECOND_PATH_SPAN: Span = Span { start: 21, end: 31 };

            #[rstest]
            #[case::second_path_escapes(PATHS_FILE, OUTSIDE_PATH, SECOND_PATH_SPAN)]
            #[case::first_path_escapes(OUTSIDE_PATH, PATHS_FILE, FIRST_PATH_SPAN)]
            #[case::absolute_path_elsewhere(PATHS_FILE, ABSOLUTE_OUTSIDE_PATH, SECOND_PATH_SPAN)]
            fn points_a_path_outside_the_repository_at_that_path(
                #[case] first: &str,
                #[case] second: &str,
                #[case] expected_span: Span,
            ) -> Result<(), TestError> {
                let paths = vec![
                    Spanned {
                        item: first.to_owned(),
                        span: FIRST_PATH_SPAN,
                    },
                    Spanned {
                        item: second.to_owned(),
                        span: SECOND_PATH_SPAN,
                    },
                ];

                let err = resolve_paths(Some(Path::new(WORKTREE)), Path::new(WORKTREE), paths)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

                assert_matches!(&err, CommandError::Located { span, .. } if *span == expected_span, "got {err:?}");

                Ok(())
            }
        }

        mod errors {
            use super::*;

            #[rstest]
            #[case::negative_limit(NEGATIVE_LIMIT_SCRIPT, NEGATIVE_LIMIT_CODE)]
            #[case::negative_skip(NEGATIVE_SKIP_SCRIPT, NEGATIVE_SKIP_CODE)]
            #[case::blank_revision(BLANK_REVISION_SCRIPT, INVALID_REVISION_CODE)]
            #[case::unknown_revision(UNKNOWN_REVISION_SCRIPT, BAD_REVISION_CODE)]
            #[case::not_a_commit(NOT_A_COMMIT_SCRIPT, NOT_A_COMMIT_CODE)]
            #[case::path_outside_repository(
                PATH_OUTSIDE_REPOSITORY_SCRIPT,
                PATH_OUTSIDE_REPOSITORY_CODE
            )]
            #[case::invalid_pathspec(INVALID_PATHSPEC_SCRIPT, INVALID_PATHSPEC_CODE)]
            fn fail_with_a_code_and_a_label_inside_the_script(
                #[case] script: &str,
                #[case] code: &str,
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;

                pin_pwd(&mut plugin, repo.path());

                let labeled = eval_labeled_error(&mut plugin, script, repo_input(repo.path())?)?;

                assert_eq!(labeled.code.as_deref(), Some(code));
                assert_label_in_script!(&labeled, script);

                Ok(())
            }

            #[rstest]
            fn reports_an_unborn_head_with_its_code(
                plugin_test: Result<PluginTest, FixtureError>,
                unborn_repo: Result<UnbornTestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = unborn_repo?;

                let error = plugin
                    .eval_with(COMMAND_NAME, repo_input(repo.path())?)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.into()))?;

                let code = error.code().map(|code| code.to_string());

                assert_eq!(code.as_deref(), Some(UNBORN_HEAD_CODE));

                Ok(())
            }
        }

        mod handle {
            use super::*;

            /// How a test breaks the repository behind the handle that `repo_input` builds.
            #[derive(Clone, Copy, Debug)]
            enum Breakage {
                /// Deletes the repository directory, so the handle's location is gone.
                RemoveRepository,
                /// Deletes HEAD, so a fresh registry cannot reopen the repository from its location.
                RemoveHead,
            }

            #[rstest]
            #[case::repository_gone(Breakage::RemoveRepository, HANDLE_REPOSITORY_GONE_CODE)]
            #[case::unopenable_rebuild(Breakage::RemoveHead, HANDLE_OPEN_FAILED_CODE)]
            fn fail_with_a_code_for_the_broken_handle(
                #[case] breakage: Breakage,
                #[case] code: &str,
                plugin_test: Result<PluginTest, FixtureError>,
                test_repo: Result<TestRepo, FixtureError>,
            ) -> Result<(), TestError> {
                let mut plugin = plugin_test?;
                let repo = test_repo?;
                let input = repo_input(repo.path())?;

                match breakage {
                    Breakage::RemoveRepository => fs::remove_dir_all(repo.path())?,
                    Breakage::RemoveHead => {
                        fs::remove_file(repo.path().join(GIT_DIR).join(HEAD_FILE))?;
                    }
                }

                let error = plugin
                    .eval_with(COMMAND_NAME, input)
                    .err()
                    .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.into()))?;

                let found = error.code().map(|identifier| identifier.to_string());

                assert_eq!(found.as_deref(), Some(code));

                Ok(())
            }
        }
    }

    mod commit_record {
        use super::*;

        fn sample_commit(
            body: Option<&str>,
            files: Option<Vec<FileChange>>,
        ) -> Result<CommitInfo, TestError> {
            let time = chrono::DateTime::parse_from_rfc3339(SAMPLE_TIME)
                .map_err(|err| TestError::Failure(err.to_string()))?;

            let parents = SAMPLE_PARENTS
                .iter()
                .map(|parent| gix::ObjectId::from_hex(parent.as_bytes()))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(CommitInfo {
                oid: gix::ObjectId::from_hex(SAMPLE_OID.as_bytes())?,
                parents,
                tree: gix::ObjectId::from_hex(SAMPLE_TREE.as_bytes())?,
                subject: SAMPLE_SUBJECT.to_owned(),
                body: body.map(str::to_owned),
                message: SAMPLE_MESSAGE.to_owned(),
                author: SignatureInfo {
                    name: SAMPLE_AUTHOR_NAME.to_owned(),
                    email: SAMPLE_AUTHOR_EMAIL.to_owned(),
                    time,
                },
                committer: SignatureInfo {
                    name: SAMPLE_COMMITTER_NAME.to_owned(),
                    email: SAMPLE_COMMITTER_EMAIL.to_owned(),
                    time,
                },
                files,
            })
        }

        #[rstest]
        fn nests_name_email_and_date_under_each_signature() -> Result<(), TestError> {
            let value = commit_record(sample_commit(None, None)?, Span::test_data());
            let author = record_column(&value, AUTHOR_COLUMN)?;
            let time = record_column(author, TIME_FIELD)?;

            let author_columns: Vec<&str> =
                author.as_record()?.columns().map(String::as_str).collect();

            let committer_name =
                record_column(record_column(&value, COMMITTER_COLUMN)?, NAME_FIELD)?;

            assert_eq!(author_columns, [NAME_FIELD, EMAIL_FIELD, TIME_FIELD]);
            assert_matches!(time, Value::Date { .. });
            assert_eq!(committer_name.as_str()?, SAMPLE_COMMITTER_NAME);

            Ok(())
        }

        #[rstest]
        fn lists_the_parents_as_strings() -> Result<(), TestError> {
            let value = commit_record(sample_commit(None, None)?, Span::test_data());

            let parents = record_column(&value, PARENTS_COLUMN)?
                .as_list()?
                .iter()
                .map(Value::as_str)
                .collect::<Result<Vec<&str>, _>>()?;

            assert_eq!(parents, SAMPLE_PARENTS);

            Ok(())
        }

        #[rstest]
        #[case::present(Some(SAMPLE_BODY))]
        #[case::absent(None)]
        fn maps_the_body_to_a_string_or_null(#[case] body: Option<&str>) -> Result<(), TestError> {
            let value = commit_record(sample_commit(body, None)?, Span::test_data());
            let column = record_column(&value, BODY_COLUMN)?;

            match body {
                Some(text) => assert_eq!(column.as_str()?, text),
                None => assert_matches!(column, Value::Nothing { .. }),
            }

            Ok(())
        }

        #[rstest]
        fn maps_files_to_path_and_change_records() -> Result<(), TestError> {
            let changed = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: None,
            }];

            let value = commit_record(sample_commit(None, Some(changed))?, Span::test_data());
            let files = record_column(&value, FILES_COLUMN)?.as_list()?;

            let [file] = files else {
                return Err(TestError::Failure(format!(
                    "expected one changed file, got {files:?}"
                )));
            };

            let columns: Vec<&str> = file.as_record()?.columns().map(String::as_str).collect();

            assert_eq!(columns, [PATH_FIELD, CHANGE_FIELD]);
            assert_eq!(record_column(file, CHANGE_FIELD)?.as_str()?, MODIFIED);

            Ok(())
        }

        #[rstest]
        fn omits_the_files_column_when_not_requested() -> Result<(), TestError> {
            let value = commit_record(sample_commit(None, None)?, Span::test_data());
            let columns: Vec<&str> = value.as_record()?.columns().map(String::as_str).collect();

            assert_eq!(columns, ROW_COLUMNS);

            Ok(())
        }

        #[rstest]
        fn maps_a_text_stat_to_integer_insertions_and_deletions() -> Result<(), TestError> {
            let changed = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: Some(FileStat::Text {
                    insertions: STAT_INSERTIONS,
                    deletions: STAT_DELETIONS,
                }),
            }];

            let value = commit_record(sample_commit(None, Some(changed))?, Span::test_data());
            let files = record_column(&value, FILES_COLUMN)?.as_list()?;

            let [file] = files else {
                return Err(TestError::Failure(format!(
                    "expected one changed file, got {files:?}"
                )));
            };

            assert_eq!(
                record_column(file, INSERTIONS_FIELD)?.as_int()?,
                i64::from(STAT_INSERTIONS)
            );
            assert_eq!(
                record_column(file, DELETIONS_FIELD)?.as_int()?,
                i64::from(STAT_DELETIONS)
            );

            Ok(())
        }

        #[rstest]
        fn maps_a_binary_stat_to_null_insertions_and_deletions() -> Result<(), TestError> {
            let changed = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Addition,
                stat: Some(FileStat::Binary),
            }];

            let value = commit_record(sample_commit(None, Some(changed))?, Span::test_data());
            let files = record_column(&value, FILES_COLUMN)?.as_list()?;

            let [file] = files else {
                return Err(TestError::Failure(format!(
                    "expected one changed file, got {files:?}"
                )));
            };

            assert_matches!(
                record_column(file, INSERTIONS_FIELD)?,
                Value::Nothing { .. }
            );
            assert_matches!(record_column(file, DELETIONS_FIELD)?, Value::Nothing { .. });

            Ok(())
        }

        #[rstest]
        fn maps_an_unavailable_stat_to_null_insertions_and_deletions() -> Result<(), TestError> {
            let changed = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: Some(FileStat::Unavailable),
            }];

            let value = commit_record(sample_commit(None, Some(changed))?, Span::test_data());
            let files = record_column(&value, FILES_COLUMN)?.as_list()?;

            let [file] = files else {
                return Err(TestError::Failure(format!(
                    "expected one changed file, got {files:?}"
                )));
            };

            assert_matches!(
                record_column(file, INSERTIONS_FIELD)?,
                Value::Nothing { .. }
            );
            assert_matches!(record_column(file, DELETIONS_FIELD)?, Value::Nothing { .. });

            Ok(())
        }

        #[rstest]
        fn omits_the_stat_columns_when_the_file_has_no_stat() -> Result<(), TestError> {
            let changed = vec![FileChange {
                path: PATHS_NESTED_FILE.to_owned(),
                kind: FileChangeKind::Modification,
                stat: None,
            }];

            let value = commit_record(sample_commit(None, Some(changed))?, Span::test_data());
            let files = record_column(&value, FILES_COLUMN)?.as_list()?;

            let [file] = files else {
                return Err(TestError::Failure(format!(
                    "expected one changed file, got {files:?}"
                )));
            };

            let columns: Vec<&str> = file.as_record()?.columns().map(String::as_str).collect();

            assert_eq!(columns, [PATH_FIELD, CHANGE_FIELD]);

            Ok(())
        }
    }

    mod commit_value {
        use super::*;

        #[rstest]
        fn turns_a_bad_timestamp_into_an_error_value_with_code_span_and_message()
        -> Result<(), TestError> {
            let commit = Err(GixError::Timestamp {
                id: BAD_TIMESTAMP_ID.to_owned(),
            });

            let value = commit_value(commit, ERROR_SPAN);

            let Value::Error { error, .. } = &value else {
                return Err(TestError::Failure(format!(
                    "expected an error value, got {value:?}"
                )));
            };

            let code = error.code().map(|code| code.to_string());

            assert_eq!(code.as_deref(), Some(BAD_TIMESTAMP_CODE));
            assert_eq!(value.span(), ERROR_SPAN);
            assert!(!error.to_string().is_empty(), "the error has no message");

            Ok(())
        }
    }

    mod non_negative {
        use super::*;

        #[rstest]
        fn rejects_a_negative_limit_at_its_value() -> Result<(), TestError> {
            let value = Spanned {
                item: NEGATIVE_LIMIT,
                span: LIMIT_VALUE_SPAN,
            };

            let err = non_negative(value, |limit, span| CommandError::NegativeLimit {
                limit,
                span,
            })
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NegativeLimit { span, .. } if *span == LIMIT_VALUE_SPAN,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn rejects_a_negative_skip_at_its_value() -> Result<(), TestError> {
            let value = Spanned {
                item: NEGATIVE_SKIP,
                span: SKIP_VALUE_SPAN,
            };

            let err = non_negative(value, |skip, span| CommandError::NegativeSkip {
                skip,
                span,
            })
            .err()
            .ok_or_else(|| TestError::Failure(EXPECTED_ERROR.to_owned()))?;

            assert_matches!(
                &err,
                CommandError::NegativeSkip { span, .. } if *span == SKIP_VALUE_SPAN,
                "got {err:?}"
            );

            Ok(())
        }
    }

    mod log_failure {
        use super::*;

        #[rstest]
        fn points_an_unresolved_revision_among_several_at_its_own_span() -> Result<(), TestError> {
            let source = GixError::Revision {
                expression: UNKNOWN_REVISION.to_owned(),
                source: gix_error()?,
            };

            let revisions = [
                Spanned {
                    item: HEAD_REVISION.to_owned(),
                    span: REVISION_SPAN,
                },
                Spanned {
                    item: UNKNOWN_REVISION.to_owned(),
                    span: SECOND_REVISION_SPAN,
                },
            ];

            let err = log_failure(source, &revisions, CALL_SPAN);

            assert_matches!(
                &err,
                CommandError::Located { span, .. } if *span == SECOND_REVISION_SPAN,
                "got {err:?}"
            );

            Ok(())
        }

        #[rstest]
        fn points_a_non_commit_revision_at_the_revision() {
            let source = GixError::NotACommit {
                expression: TREE_REVISION.to_owned(),
                kind: TREE_KIND.to_owned(),
            };

            let revisions = [Spanned {
                item: TREE_REVISION.to_owned(),
                span: REVISION_SPAN,
            }];

            let err = log_failure(source, &revisions, CALL_SPAN);

            assert_matches!(
                &err,
                CommandError::Located { span, .. } if *span == REVISION_SPAN,
                "got {err:?}"
            );
        }

        #[rstest]
        fn passes_other_failures_through() {
            let err = log_failure(
                GixError::UnbornHead {
                    name: MAIN_BRANCH.to_owned(),
                },
                &[],
                CALL_SPAN,
            );

            assert_matches!(
                &err,
                CommandError::Git(GixError::UnbornHead { .. }),
                "got {err:?}"
            );
        }
    }
}
