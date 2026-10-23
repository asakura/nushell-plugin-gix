//! Tests of the mapping of command errors to `LabeledError`s.

use super::*;

use crate::cdt::{Handle, Repository};

use gix::config::{
    Source,
    parse::{Error as ParseError, Events},
};
use nu_plugin_gix_core::{
    ConfigKey, ConfigType, Origin, RefExpect, RefTarget, RepositoryLocation, ScalarType,
    SectionFault, SectionHeader, SectionName, VariableName,
};
use nu_protocol::ShellError;
use rstest::rstest;

use std::{
    io,
    path::{Path, PathBuf},
    str,
    sync::Arc,
};

const MISSING_PATH: &str = "/nonexistent/gix-test";
const REF_NAME: &str = "refs/heads/main";
const SHORTHAND_NAME: &str = "main";

/// The object id the reference actually held after a concurrent move.
const ACTUAL_OID: &str = "fedcba9876543210fedcba9876543210fedcba98";

#[derive(Debug, thiserror::Error)]
enum TestError {
    #[error("Test failure: {0}")]
    Failure(String),
}

/// The error `RepositoryLocation::discover` gives for a path outside any repository.
fn missing_repository() -> Result<GixError, TestError> {
    RepositoryLocation::discover(Path::new(MISSING_PATH))
        .err()
        .ok_or_else(|| TestError::Failure("expected an error".into()))
}

/// A stand-in `gix::Error` to use as the `source` of a git failure.
fn gix_error() -> gix::Error {
    gix::Error::from_error(io::Error::other("simulated git failure"))
}

/// The origin of a value read from a file, as the config store records it.
fn file_origin() -> Arc<Origin> {
    Arc::new(Origin {
        path: Some(PathBuf::from("/p/config")),
        source: Source::Local,
    })
}

/// The key `a.b`, as `gix config a.b` reads it.
fn a_b_key() -> ConfigKey {
    ConfigKey::new(SectionName::from("a"), None, VariableName::from("b"))
}

/// A parse failure of gix's config parser, the `source` of `ConfigSectionUnreadable`.
fn parse_failure() -> Result<ParseError, TestError> {
    let Err(failure) = Events::from_bytes(b"[", None) else {
        return Err(TestError::Failure("expected a parse error".to_owned()));
    };
    Ok(failure)
}

/// The failure the engine reports when it cannot answer a request.
fn engine_failure() -> ShellError {
    ShellError::NushellFailed {
        msg: "engine unavailable".to_owned(),
    }
}

/// Unit tests for the `into_labeled` mapping an evaluated script cannot observe:
/// the exact span each error points at and the label/help mapping of variants the
/// command surface cannot raise.
///
/// `WrongInput` is here because the engine rejects non-handle values before they
/// reach a command, and no command produces a second handle type to pipe in yet.
mod into_labeled {
    use super::*;

    #[rstest]
    fn points_wrong_input_at_the_input() {
        let input_span = Span::new(3, 9);

        let err = CommandError::WrongInput(WrongInput {
            expected: <Repository as Handle>::TYPE_NAME,
            hint: "pipe the output of `gix open` into this command",
            got: "int".to_owned(),
            span: input_span,
        });

        let labeled = err.into_labeled(Span::test_data());

        assert_eq!(labeled.msg, "expected a `git repository` value");
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(input_span));

        assert_eq!(
            labeled.help.as_deref(),
            Some("pipe the output of `gix open` into this command")
        );
    }

    #[rstest]
    fn points_an_invalid_side_type_at_the_value() {
        let input_span = Span::new(3, 9);
        let labeled = CommandError::InvalidSideType {
            got: "int".to_owned(),
            span: input_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(input_span)
        );
        assert_eq!(
            labeled.help.as_deref(),
            Some("use `head`, `index`, `worktree`, or a revision")
        );
    }

    #[rstest]
    fn points_an_empty_revision_at_the_revision() {
        let revision_span = Span::new(3, 9);
        let labeled = CommandError::EmptyRevision {
            span: revision_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(revision_span)
        );
        assert_eq!(
            labeled.help.as_deref(),
            Some("provide a non-empty revision expression")
        );
    }

    #[rstest]
    fn labels_an_empty_revision_with_its_own_text() {
        let labeled = CommandError::EmptyRevision {
            span: Span::test_data(),
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.text.as_str()),
            Some("the revision expression is empty")
        );
    }

    #[rstest]
    fn labels_an_invalid_object_id_with_its_own_text() {
        let labeled = CommandError::InvalidObjectId {
            expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
            span: Span::test_data(),
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.text.as_str()),
            Some("contains characters that are not hexadecimal digits")
        );
    }

    #[rstest]
    fn helps_an_invalid_object_id_with_both_hex_cases() {
        let labeled = CommandError::InvalidObjectId {
            expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
            span: Span::test_data(),
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.help.as_deref(),
            Some("use only the digits 0-9 and the letters a-f or A-F")
        );
    }

    #[rstest]
    fn points_a_non_full_object_id_at_the_value() {
        let value_span = Span::new(3, 9);
        let labeled = CommandError::NotAFullObjectId {
            value: "0123456".to_owned(),
            span: value_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(value_span)
        );
    }

    #[rstest]
    fn points_an_invalid_side_record_at_the_record() {
        let record_span = Span::new(3, 9);
        let labeled =
            CommandError::InvalidSideRecord { span: record_span }.into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(record_span)
        );
        assert_eq!(
            labeled.help.as_deref(),
            Some("use a record with exactly one `rev` field")
        );
    }

    #[rstest]
    fn points_an_invalid_side_field_at_the_field_value() {
        let field_span = Span::new(3, 9);
        let labeled = CommandError::InvalidSideField {
            field: "rev",
            got: "int".to_owned(),
            span: field_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(field_span)
        );
        assert_eq!(
            labeled.help.as_deref(),
            Some("the `rev` field must contain a revision string")
        );
    }

    #[rstest]
    fn reports_a_deleted_repository_with_its_message() {
        let err = CommandError::from(GixError::RepositoryGone {
            git_dir: PathBuf::from("/gone/.git"),
        });

        let labeled = err.into_labeled(Span::test_data());

        assert_eq!(
            labeled.msg,
            "git repository at '/gone/.git' was moved or deleted"
        );
        assert_eq!(labeled.labels.len(), 1);
    }

    #[rstest]
    fn points_a_handle_whose_repository_is_gone_at_the_handle() {
        let handle_span = Span::new(3, 9);

        let labeled = CommandError::handle_repository_gone(
            PathBuf::from("/gone/.git"),
            handle_span,
            GixError::RepositoryGone {
                git_dir: PathBuf::from("/gone/.git"),
            },
        )
        .into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(handle_span));

        assert_eq!(
            labeled.msg,
            "the git repository at '/gone/.git' behind this handle was moved or deleted"
        );

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some("this handle's repository is no longer at its location")
        );

        assert_eq!(
            labeled.help.as_deref(),
            Some("run `gix open` again to get a handle to the repository's current location")
        );
    }

    #[rstest]
    fn points_a_handle_that_cannot_be_reopened_at_the_handle() {
        let handle_span = Span::new(3, 9);

        let labeled = CommandError::handle_open_failed(
            PathBuf::from(MISSING_PATH),
            handle_span,
            GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source: gix_error(),
            },
        )
        .into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(handle_span));

        assert_eq!(
            labeled.msg,
            format!("could not reopen the git repository at '{MISSING_PATH}' behind this handle")
        );

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some("this handle's repository can no longer be opened; it may be corrupt")
        );
    }

    #[rstest]
    fn labels_unopenable_repository_as_possibly_corrupt() -> Result<(), TestError> {
        let source = gix::open(MISSING_PATH)
            .err()
            .ok_or_else(|| TestError::Failure("expected an error".to_owned()))?;

        let err = CommandError::from(GixError::Open {
            path: PathBuf::from(MISSING_PATH),
            source,
        });

        let labeled = err.into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some("the repository was found but could not be opened; it may be corrupt")
        );

        Ok(())
    }

    #[rstest]
    fn does_not_repeat_the_headline_in_help() -> Result<(), TestError> {
        let err = CommandError::not_a_repository(
            PathBuf::from(MISSING_PATH),
            Span::test_data(),
            missing_repository()?,
        );

        let labeled = err.into_labeled(Span::test_data());

        assert!(
            labeled
                .help
                .as_deref()
                .is_none_or(|help| !help.contains("no git repository found")),
            "got {:?}",
            labeled.help
        );

        Ok(())
    }

    #[rstest]
    fn points_an_open_failure_at_the_flag() {
        let flag_span = Span::new(3, 9);

        let err = CommandError::open_failed(
            PathBuf::from(MISSING_PATH),
            flag_span,
            GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source: gix_error(),
            },
        );

        let labeled = err.into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
        assert_eq!(labeled.code.as_deref(), Some(OPEN_FAILED_CODE));
    }

    #[rstest]
    fn points_a_non_repository_at_the_flag_with_its_code_and_label() -> Result<(), TestError> {
        let flag_span = Span::new(3, 9);
        let labeled = CommandError::not_a_repository(
            PathBuf::from(MISSING_PATH),
            flag_span,
            missing_repository()?,
        )
        .into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some("no git repository found at this path")
        );
        assert_eq!(labeled.code.as_deref(), Some(NOT_A_REPOSITORY_CODE));

        Ok(())
    }

    #[rstest]
    fn points_a_located_failure_at_its_span() {
        let revision_span = Span::new(3, 9);
        let labeled = CommandError::Located {
            span: revision_span,
            source: GixError::Revision {
                expression: "nope".to_owned(),
                source: gix_error(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(revision_span));
    }

    #[rstest]
    fn points_an_unknown_config_type_at_its_value() {
        let value_span = Span::new(3, 9);

        let labeled = CommandError::UnknownConfigType {
            given: "bogus".to_owned(),
            span: value_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(UNKNOWN_CONFIG_TYPE_CODE));
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(value_span));
        assert!(labeled.msg.contains("bogus"), "got {:?}", labeled.msg);
    }

    #[rstest]
    fn points_type_without_a_key_at_the_flag() {
        let flag_span = Span::new(3, 9);

        let labeled =
            CommandError::ConfigTypeNeedsKey { span: flag_span }.into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(CONFIG_TYPE_NEEDS_KEY_CODE));
        assert!(
            labeled
                .labels
                .first()
                .is_some_and(|label| !label.text.is_empty()),
            "got {:?}",
            labeled.labels
        );
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(flag_span));
    }

    #[rstest]
    fn points_type_on_a_section_at_its_key() {
        let key_span = Span::new(3, 9);

        let labeled = CommandError::ConfigTypeNeedsValue {
            key: "core".to_owned(),
            span: key_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(CONFIG_TYPE_NEEDS_VALUE_CODE));
        assert!(labeled.msg.contains("core"), "got {:?}", labeled.msg);
        assert!(
            labeled
                .labels
                .first()
                .is_some_and(|label| !label.text.is_empty()),
            "got {:?}",
            labeled.labels
        );
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(key_span));
    }

    #[rstest]
    fn points_a_missing_config_key_at_its_key() {
        let key_span = Span::new(3, 9);

        let labeled = CommandError::ConfigKeyNotFound {
            key: "user.nope".to_owned(),
            span: key_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(CONFIG_KEY_NOT_FOUND_CODE));
        assert!(
            labeled
                .labels
                .first()
                .is_some_and(|label| !label.text.is_empty()),
            "got {:?}",
            labeled.labels
        );
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(key_span));
    }

    #[rstest]
    fn labels_an_unresolved_revision_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::Revision {
                expression: "nope".to_owned(),
                source: gix_error(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(UNRESOLVED_REVISION_LABEL)
        );
    }

    #[rstest]
    fn labels_a_non_commit_revision_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::NotACommit {
                expression: "HEAD^{tree}".to_owned(),
                kind: "tree".to_owned(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(NOT_A_COMMIT_LABEL)
        );
    }

    #[rstest]
    fn labels_a_path_outside_the_repository_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::PathOutsideRepository {
                path: "../outside".to_owned(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(PATH_OUTSIDE_REPOSITORY_LABEL)
        );
    }

    #[rstest]
    fn labels_an_out_of_range_short_length_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::ShortLen {
                len: 3,
                detail: "expected 4..=40".to_owned(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(SHORT_LEN_OUT_OF_RANGE_LABEL)
        );
    }

    #[rstest]
    #[case::invalid_name(GixError::InvalidRefName {
            name: SHORTHAND_NAME.to_owned(),
            detail: "expected a full name".to_owned(),
        })]
    #[case::moved(GixError::RefMoved {
            name: REF_NAME.to_owned(),
            expected: RefExpect::Missing,
            actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
        })]
    fn points_a_ref_update_failure_at_its_span(#[case] source: GixError) {
        let span = Span::new(3, 9);
        let labeled = CommandError::Located { span, source }.into_labeled(Span::test_data());

        assert_eq!(labeled.labels.first().map(|l| l.span), Some(span));
    }

    #[rstest]
    fn labels_an_invalid_ref_name_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::InvalidRefName {
                name: SHORTHAND_NAME.to_owned(),
                detail: "expected a full name".to_owned(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(INVALID_REF_NAME_LABEL)
        );
    }

    #[rstest]
    fn labels_a_moved_reference_with_its_reason() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::RefMoved {
                name: REF_NAME.to_owned(),
                expected: RefExpect::Missing,
                actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|l| l.text.as_str()),
            Some(REF_MOVED_LABEL)
        );
    }

    #[rstest]
    fn keeps_the_located_headline_out_of_the_help() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::Revision {
                expression: "nope".to_owned(),
                source: gix_error(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.msg, "could not resolve revision 'nope'");
        assert!(
            labeled
                .help
                .as_deref()
                .is_none_or(|help| !help.contains("could not resolve revision")),
            "got {:?}",
            labeled.help
        );
    }

    #[rstest]
    fn points_a_negative_limit_at_its_span() {
        let limit_span = Span::new(3, 9);
        let labeled = CommandError::NegativeLimit {
            limit: -1,
            span: limit_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(limit_span)
        );
    }

    #[rstest]
    fn labels_a_negative_limit_with_its_code_and_reason() {
        const NEGATIVE_LIMIT_REASON: &str = "expected a non-negative integer";

        let labeled = CommandError::NegativeLimit {
            limit: -1,
            span: Span::test_data(),
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(NEGATIVE_LIMIT_CODE));
        assert_eq!(
            labeled.labels.first().map(|label| label.text.as_str()),
            Some(NEGATIVE_LIMIT_REASON)
        );
    }

    #[rstest]
    fn points_an_argument_error_at_its_span() {
        let argument_span = Span::new(3, 9);
        let labeled = CommandError::InvalidArgument {
            source: Box::new(engine_failure()),
            span: argument_span,
        }
        .into_labeled(Span::test_data());

        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(argument_span)
        );
    }

    #[rstest]
    fn has_no_label_for_a_timestamp_error() {
        let err = CommandError::Git(GixError::Timestamp {
            id: "abc".to_owned(),
        });

        let labeled = err.into_labeled(Span::test_data());

        assert!(labeled.labels.is_empty());
    }

    const ARGUMENT_SPAN: Span = Span { start: 3, end: 9 };

    #[rstest]
    #[case::not_a_full_object_id(
            CommandError::NotAFullObjectId {
                value: "0123456".to_owned(),
                span: ARGUMENT_SPAN,
            },
            NOT_A_FULL_OBJECT_ID_CODE
        )]
    #[case::invalid_object_id(
            CommandError::InvalidObjectId {
                expression: "0123456789abcdef0123456789abcdef0123456z".to_owned(),
                span: ARGUMENT_SPAN,
            },
            INVALID_REVISION_CODE
        )]
    #[case::empty_revision(
            CommandError::EmptyRevision {
                span: ARGUMENT_SPAN,
            },
            INVALID_REVISION_CODE
        )]
    #[case::missing_expect(
            CommandError::MissingExpect {
                span: ARGUMENT_SPAN,
            },
            MISSING_EXPECT_CODE
        )]
    #[case::negative_short_len(
            CommandError::NegativeShortLen {
                len: -1,
                span: ARGUMENT_SPAN,
            },
            INVALID_SHORT_LEN_CODE
        )]
    #[case::negative_skip(
            CommandError::NegativeSkip {
                skip: -1,
                span: ARGUMENT_SPAN,
            },
            NEGATIVE_SKIP_CODE
        )]
    fn carries_the_code_and_the_span_of_each_argument_error(
        #[case] error: CommandError,
        #[case] code: &str,
    ) {
        let labeled = error.into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(code));
        assert_eq!(
            labeled.labels.first().map(|label| label.span),
            Some(ARGUMENT_SPAN)
        );
    }
}

/// Unit tests for the variant-to-code mapping of errors that no script-level test asserts.
mod codes {
    use super::*;

    /// A byte that is not valid UTF-8 on its own.
    const NOT_UTF8_BYTE: u8 = 0xff;

    // No command parses sides yet, so these variants cannot be triggered by a script.
    #[rstest]
    #[case::invalid_side_type(
            CommandError::InvalidSideType {
                got: "int".to_owned(),
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
    #[case::invalid_side_record(
            CommandError::InvalidSideRecord {
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
    #[case::invalid_side_field(
            CommandError::InvalidSideField {
                field: "rev",
                got: "int".to_owned(),
                span: Span::test_data(),
            },
            INVALID_SIDE_CODE
        )]
    fn side_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
        let labeled = error.into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(code));
    }

    // The engine cannot be made to fail from a script, so these variants are checked directly.
    #[rstest]
    #[case::current_dir(
            CommandError::CurrentDir {
                source: Box::new(engine_failure()),
                span: Span::test_data(),
            },
            CURRENT_DIR_CODE
        )]
    #[case::help_unavailable(
            CommandError::HelpUnavailable {
                source: Box::new(engine_failure()),
                span: Span::test_data(),
            },
            HELP_UNAVAILABLE_CODE
        )]
    fn engine_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
        let labeled = error.into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(code));
    }

    #[rstest]
    #[case::open(
            CommandError::from(GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source: gix_error(),
            }),
            OPEN_FAILED_CODE
        )]
    #[case::revision(
            CommandError::from(GixError::Revision {
                expression: "nope".to_owned(),
                source: gix_error(),
            }),
            BAD_REVISION_CODE
        )]
    #[case::shorten(
            CommandError::from(GixError::Shorten {
                expression: "nope".to_owned(),
                source: gix_error(),
            }),
            BAD_REVISION_CODE
        )]
    #[case::head(CommandError::from(GixError::Head(gix_error())), BAD_REVISION_CODE)]
    #[case::walk(CommandError::from(GixError::Walk(gix_error())), WALK_FAILED_CODE)]
    #[case::commit(
            CommandError::from(GixError::Commit {
                id: "abc".to_owned(),
                source: gix_error(),
            }),
            COMMIT_UNREADABLE_CODE
        )]
    #[case::repository_gone(
            CommandError::from(GixError::RepositoryGone {
                git_dir: PathBuf::from("/gone/.git"),
            }),
            REPOSITORY_GONE_CODE
        )]
    #[case::spawn(
        CommandError::from(GixError::Spawn(io::Error::other("no thread"))),
        IO_CODE
    )]
    #[case::walker_panicked(CommandError::from(GixError::WalkerPanicked), WALKER_PANICKED_CODE)]
    #[case::ref_read(
        CommandError::from(GixError::RefRead(gix_error())),
        REF_READ_FAILED_CODE
    )]
    #[case::dangling_ref(
            CommandError::from(GixError::DanglingRef {
                name: "refs/heads/main".to_owned(),
            }),
            REF_READ_FAILED_CODE
        )]
    #[case::ref_moved(
            CommandError::from(GixError::RefMoved {
                name: "refs/heads/main".to_owned(),
                expected: RefExpect::Missing,
                actual: Some(RefTarget::Object(ACTUAL_OID.to_owned())),
            }),
            REF_MOVED_CODE
        )]
    #[case::ref_update(
            CommandError::from(GixError::RefUpdate {
                name: "refs/heads/main".to_owned(),
                source: gix_error(),
            }),
            REF_UPDATE_FAILED_CODE
        )]
    #[case::invalid_ref_name(
            CommandError::from(GixError::InvalidRefName {
                name: "main".to_owned(),
                detail: "missing the `refs/` prefix".to_owned(),
            }),
            INVALID_REF_NAME_CODE
        )]
    #[case::pathspec(
            CommandError::from(GixError::Pathspec {
                spec: ":(nonsense)x".to_owned(),
                source: gix_error(),
            }),
            INVALID_PATHSPEC_CODE
        )]
    #[case::pathspec_attributes(
            CommandError::from(GixError::PathspecAttributes {
                spec: ":(attr:linguist-vendored)x".to_owned(),
            }),
            INVALID_PATHSPEC_CODE
        )]
    #[case::short_len(
            CommandError::from(GixError::ShortLen {
                len: 3,
                detail: "below the minimum of 4".to_owned(),
            }),
            INVALID_SHORT_LEN_CODE
        )]
    #[case::not_a_commit(
            CommandError::from(GixError::NotACommit {
                expression: "HEAD^{tree}".to_owned(),
                kind: "tree".to_owned(),
            }),
            NOT_A_COMMIT_CODE
        )]
    #[case::path_outside_repository(
            CommandError::from(GixError::PathOutsideRepository {
                path: "../outside".to_owned(),
            }),
            PATH_OUTSIDE_REPOSITORY_CODE
        )]
    fn git_errors_carry_stable_codes(#[case] error: CommandError, #[case] code: &str) {
        let labeled = error.into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(code));
    }

    /// A value that is not UTF-8, as a `GixError::ConfigValueNotUtf8` carries it.
    fn value_not_utf8() -> Result<GixError, TestError> {
        let Err(source) = str::from_utf8(&[NOT_UTF8_BYTE]) else {
            return Err(TestError::Failure("expected a UTF-8 error".to_owned()));
        };
        Ok(GixError::ConfigValueNotUtf8 {
            key: a_b_key(),
            source,
            origin: file_origin(),
        })
    }

    /// A subsection that is not UTF-8, as a `GixError::ConfigSubsectionNotUtf8` carries it.
    fn subsection_not_utf8() -> Result<GixError, TestError> {
        let Err(source) = str::from_utf8(&[NOT_UTF8_BYTE]) else {
            return Err(TestError::Failure("expected a UTF-8 error".to_owned()));
        };
        Ok(GixError::ConfigSubsectionNotUtf8 {
            key: a_b_key(),
            source,
            origin: file_origin(),
        })
    }

    /// A section whose bytes cannot be parsed again, as a `GixError::ConfigSectionUnreadable` carries it.
    fn section_unreadable() -> Result<GixError, TestError> {
        Ok(GixError::ConfigSectionUnreadable {
            section: SectionHeader {
                section: SectionName::from("a"),
                subsection: None,
            },
            source: parse_failure()?,
        })
    }

    /// Every config variant is asserted here once: its code, its label and the span it points at
    /// when a command attributes it to an argument.
    #[rstest]
    #[case::invalid_config_key(
            Ok(GixError::InvalidConfigKey {
                key: "nodot".to_owned(),
            }),
            INVALID_CONFIG_KEY_CODE,
            INVALID_CONFIG_KEY_LABEL
        )]
    #[case::config_value_not_utf8(
        value_not_utf8(),
        CONFIG_VALUE_NOT_UTF8_CODE,
        CONFIG_VALUE_NOT_UTF8_LABEL
    )]
    #[case::config_value_conversion(
            Ok(GixError::ConfigValueConversion {
                key: a_b_key(),
                kind: ConfigType::Scalar(ScalarType::Int),
                source: gix_error(),
                origin: file_origin(),
            }),
            CONFIG_VALUE_CONVERSION_CODE,
            CONFIG_VALUE_CONVERSION_LABEL
        )]
    #[case::config_value_missing(
            Ok(GixError::ConfigValueMissing {
                key: a_b_key(),
                kind: ConfigType::Scalar(ScalarType::Int),
                origin: file_origin(),
            }),
            CONFIG_VALUE_MISSING_CODE,
            CONFIG_VALUE_MISSING_LABEL
        )]
    #[case::install_dir_unavailable(
            Ok(GixError::InstallDirUnavailable {
                key: a_b_key(),
                source: Arc::new(gix_error()),
                origin: file_origin(),
            }),
            INSTALL_DIR_UNAVAILABLE_CODE,
            INSTALL_DIR_UNAVAILABLE_LABEL
        )]
    #[case::config_path_not_utf8(
            Ok(GixError::ConfigPathNotUtf8 {
                key: a_b_key(),
                origin: file_origin(),
            }),
            CONFIG_PATH_NOT_UTF8_CODE,
            CONFIG_PATH_NOT_UTF8_LABEL
        )]
    #[case::config_key_reset(
            Ok(GixError::ConfigKeyReset {
                key: a_b_key(),
                origin: file_origin(),
            }),
            CONFIG_KEY_RESET_CODE,
            CONFIG_KEY_RESET_LABEL
        )]
    #[case::config_subsection_not_utf8(
        subsection_not_utf8(),
        CONFIG_SUBSECTION_NOT_UTF8_CODE,
        CONFIG_SUBSECTION_NOT_UTF8_LABEL
    )]
    #[case::config_section_unreadable(
        section_unreadable(),
        CONFIG_SECTION_UNREADABLE_CODE,
        CONFIG_SECTION_UNREADABLE_LABEL
    )]
    #[case::config_section_malformed(
            Ok(GixError::ConfigSectionMalformed {
                section: SectionHeader {
                section: SectionName::from("a"),
                subsection: None,
            },
                fault: SectionFault::NameWithoutValue(VariableName::from("answer")),
            }),
            CONFIG_SECTION_MALFORMED_CODE,
            CONFIG_SECTION_MALFORMED_LABEL
        )]
    fn config_errors_carry_code_label_and_span(
        #[case] error: Result<GixError, TestError>,
        #[case] code: &str,
        #[case] label: &str,
    ) -> Result<(), TestError> {
        let key_span = Span::new(3, 6);

        let labeled = CommandError::Located {
            span: key_span,
            source: error?,
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(code));
        assert_eq!(labeled.labels.first().map(|l| l.text.as_str()), Some(label));
        assert_eq!(labeled.labels.first().map(|l| l.span), Some(key_span));

        Ok(())
    }

    #[rstest]
    fn config_section_malformed_message_names_the_variable() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::ConfigSectionMalformed {
                section: SectionHeader {
                    section: SectionName::from("gixtest"),
                    subsection: None,
                },
                fault: SectionFault::NameWithoutValue(VariableName::from("answer")),
            },
        }
        .into_labeled(Span::test_data());

        assert!(labeled.msg.contains("answer"), "{}", labeled.msg);
        assert!(labeled.help.is_none(), "got {:?}", labeled.help);
    }

    #[rstest]
    fn located_shorten_failures_carry_the_git_code() {
        let labeled = CommandError::Located {
            span: Span::test_data(),
            source: GixError::Shorten {
                expression: "nope".to_owned(),
                source: gix_error(),
            },
        }
        .into_labeled(Span::test_data());

        assert_eq!(labeled.code.as_deref(), Some(BAD_REVISION_CODE));
    }

    #[rstest]
    fn unopenable_handles_carry_the_open_failed_code() {
        let handle_span = Span::test_data();

        let unopenable = CommandError::handle_open_failed(
            PathBuf::from(MISSING_PATH),
            handle_span,
            GixError::Open {
                path: PathBuf::from(MISSING_PATH),
                source: gix_error(),
            },
        );

        assert_eq!(
            unopenable.into_labeled(handle_span).code.as_deref(),
            Some(OPEN_FAILED_CODE)
        );
    }
}

mod documentation {
    use super::*;

    use crate::GixPlugin;
    use crate::commands::config::{CONFIG_COMMAND, LIST_COMMAND};

    use nu_plugin::Plugin as _;

    /// The codes a command's `extra_description` lists after `Error codes: `.
    fn listed_codes<'a>(name: &str, extra_description: &'a str) -> Result<Vec<&'a str>, TestError> {
        extra_description
            .lines()
            .find_map(|line| line.strip_prefix(CODES_LINE_PREFIX))
            .ok_or_else(|| TestError::Failure(format!("`{name}` lists no `Error codes:` line")))
            .map(|codes| codes.split(", ").collect())
    }

    #[rstest]
    fn every_documented_code_is_known() -> Result<(), TestError> {
        for command in GixPlugin::default().commands() {
            for code in listed_codes(command.name(), command.extra_description())? {
                assert!(
                    ALL_CODES.contains(&code),
                    "`{code}` is documented but not in ALL_CODES"
                );
            }
        }

        Ok(())
    }

    #[rstest]
    fn config_code_lists_are_known_codes() {
        for code in CONFIG_LIST_CODES.iter().chain(CONFIG_KEY_CODES) {
            assert!(
                ALL_CODES.contains(code),
                "`{code}` is listed but not in ALL_CODES"
            );
        }
        for code in CONFIG_KEY_CODES {
            assert!(
                !CONFIG_LIST_CODES.contains(code),
                "`{code}` is in both code lists"
            );
        }
    }

    #[rstest]
    fn config_lists_every_list_code() -> Result<(), TestError> {
        let plugin = GixPlugin::default();
        let commands = plugin.commands();
        let help_of = |name: &str| -> Result<String, TestError> {
            commands
                .iter()
                .find(|command| command.name() == name)
                .map(|command| command.extra_description().to_owned())
                .ok_or_else(|| TestError::Failure(format!("no command `{name}`")))
        };
        let config_help = help_of(CONFIG_COMMAND)?;
        let list_help = help_of(LIST_COMMAND)?;
        let config_codes = listed_codes(CONFIG_COMMAND, &config_help)?;
        let list_codes = listed_codes(LIST_COMMAND, &list_help)?;

        for code in &list_codes {
            assert!(
                config_codes.contains(code),
                "`{code}` is missing from config"
            );
        }
        for code in CONFIG_KEY_CODES {
            assert!(
                config_codes.contains(code),
                "`{code}` is missing from config"
            );
            assert!(!list_codes.contains(code), "`{code}` is listed by list");
        }

        Ok(())
    }

    #[rstest]
    fn list_codes_exclude_the_conversion_code() {
        assert!(!CONFIG_LIST_CODES.contains(&CONFIG_VALUE_CONVERSION_CODE));
        assert!(CONFIG_KEY_CODES.contains(&CONFIG_VALUE_CONVERSION_CODE));
        assert!(CONFIG_LIST_CODES.contains(&CONFIG_PATH_NOT_UTF8_CODE));
        assert!(!CONFIG_KEY_CODES.contains(&CONFIG_PATH_NOT_UTF8_CODE));
    }
}

mod error_codes_line {
    use super::*;

    #[rstest]
    fn joins_groups() {
        assert_eq!(
            error_codes_line(&[&["gix::a", "gix::b"], &["gix::c"]]),
            "Error codes: gix::a, gix::b, gix::c"
        );
    }
}
