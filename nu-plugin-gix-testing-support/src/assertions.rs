//! Assertion helpers that run a script against a plugin and inspect the value or labeled error it produces.

use nu_plugin_test_support::PluginTest;
use nu_protocol::{LabeledError, PipelineData, ShellError, Span, Value};

const EXPECTED_ERROR: &str = "expected an error";
const EXPECTED_LABELED: &str = "expected a labeled error";
const EXPECTED_LABEL: &str = "expected a label";
const EMPTY_LABEL_TEXT: &str = "the label has no text";
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
pub fn raised_labeled(
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

/// Asserts that the first label of `labeled` has text and points inside `script`.
///
/// # Errors
///
/// Returns [`AssertionError::Failure`] when `labeled` has no label, the label has no text, or its span
/// does not lie inside `script`.
pub fn assert_label_in_script(labeled: &LabeledError, script: &str) -> Result<(), AssertionError> {
    let label = labeled
        .labels
        .first()
        .ok_or_else(|| AssertionError::Failure(EXPECTED_LABEL.to_owned()))?;

    if label.text.is_empty() {
        return Err(AssertionError::Failure(EMPTY_LABEL_TEXT.to_owned()));
    }
    if label.span.start >= label.span.end || label.span.end > script.len() {
        return Err(AssertionError::Failure(format!(
            "label {:?} is outside the script",
            label.span
        )));
    }

    Ok(())
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
