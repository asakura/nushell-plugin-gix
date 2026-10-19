//! Assertion helpers that run a script against a plugin and inspect the value or labeled error it produces.

use nu_plugin_test_support::PluginTest;
use nu_protocol::{LabeledError, PipelineData, ShellError, Span, Value};

const EXPECTED_ERROR: &str = "expected an error";
const EXPECTED_LABELED: &str = "expected a labeled error";
const EXPECTED_ROW: &str = "expected at least one row";

/// Why an assertion helper could not produce the value it was asked for.
#[derive(Debug, thiserror::Error)]
pub enum AssertionError {
    /// The engine failed while running the script or while collecting its output.
    #[error(transparent)]
    Shell(#[from] Box<ShellError>),

    /// The value, label, column or row is absent or has the wrong shape.
    #[error("Test failure: {0}")]
    Failure(String),
}

impl From<ShellError> for AssertionError {
    fn from(error: ShellError) -> Self {
        Self::Shell(Box::new(error))
    }
}

/// Evaluates `script` with `input` as its pipeline input and returns the value it produces.
///
/// # Errors
///
/// Returns [`AssertionError::Shell`] when the script fails or its output cannot be collected.
pub fn eval_value(
    plugin: &mut PluginTest,
    script: &str,
    input: PipelineData,
) -> Result<Value, AssertionError> {
    let value = plugin
        .eval_with(script, input)?
        .into_value(Span::test_data())?;

    Ok(value)
}

/// Evaluates `script` with `input` and returns the labeled error it raises.
///
/// # Errors
///
/// Returns [`AssertionError::Failure`] when the script succeeds or raises an error that is not labeled.
pub fn eval_labeled_error(
    plugin: &mut PluginTest,
    script: &str,
    input: PipelineData,
) -> Result<LabeledError, AssertionError> {
    let error = plugin
        .eval_with(script, input)
        .err()
        .ok_or_else(|| AssertionError::Failure(EXPECTED_ERROR.to_owned()))?;

    match error {
        ShellError::LabeledError(labeled) => Ok(*labeled),
        other => Err(AssertionError::Failure(format!(
            "{EXPECTED_LABELED}, got {other:?}"
        ))),
    }
}

/// Asserts that the first label of a [`LabeledError`] has text and a non-empty span inside `script`.
///
/// Expands in place like [`assert!`], so a failure reports the calling test's line.
///
/// # Panics
///
/// Panics when the error has no label, its first label has no text, or that label's span is empty or
/// extends past the end of `script`.
#[macro_export]
macro_rules! assert_label_in_script {
    ($labeled:expr, $script:expr $(,)?) => {{
        let labeled: &::nu_protocol::LabeledError = $labeled;
        let script: &str = $script;
        let label = labeled.labels.first();

        assert!(
            label.is_some_and(|label| !label.text.is_empty()),
            "expected a first label with text, got {:?}",
            labeled.labels
        );
        assert!(
            label.is_some_and(
                |label| label.span.start < label.span.end && label.span.end <= script.len()
            ),
            "the first label of {:?} is outside the script",
            labeled.labels
        );
    }};
}

/// Reads the `column` of a record value.
///
/// # Errors
///
/// Returns [`AssertionError::Shell`] when `value` is not a record, and [`AssertionError::Failure`] when
/// the record has no such column.
#[must_use = "returns the column value; discarding it skips the read"]
pub fn record_column<'value>(
    value: &'value Value,
    column: &str,
) -> Result<&'value Value, AssertionError> {
    value
        .as_record()?
        .get(column)
        .ok_or_else(|| AssertionError::Failure(format!("missing the `{column}` column")))
}

/// Reads the first row of a table value.
///
/// # Errors
///
/// Returns [`AssertionError::Shell`] when `table` is not a list, and [`AssertionError::Failure`] when it
/// has no rows.
#[must_use = "returns the first row; discarding it skips the read"]
pub fn first_row(table: &Value) -> Result<&Value, AssertionError> {
    table
        .as_list()?
        .first()
        .ok_or_else(|| AssertionError::Failure(EXPECTED_ROW.to_owned()))
}
