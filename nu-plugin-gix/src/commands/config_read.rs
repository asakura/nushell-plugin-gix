//! Reads the configuration of a repository as Nushell values: the occurrences a key resolves to, their typed or untyped
//! values, and the records of sections and subsections, including [`everything`], the whole configuration as one
//! nested record built from the core tree, [`rows`], the flat table of every occurrence, and [`typed_setting`], one
//! variable read as a type.

#[cfg(test)]
#[path = "config_read_tests.rs"]
mod tests;

use super::config_known::{self, KnownKey, Multi, ValueKind};

use nu_plugin_gix_core as git;
use nu_protocol::{Record, Span, Value, record};

use std::{iter, path::PathBuf, str::Utf8Error, sync::Arc};

/// The column of the key in the table.
const KEY_COLUMN: &str = "key";
/// The column of the value in the table.
const VALUE_COLUMN: &str = "value";
/// The column of the scope in the table.
const SCOPE_COLUMN: &str = "scope";
/// The column of the file in the table.
const FILE_COLUMN: &str = "file";

/// What a key resolves to: the values that count, or the last value that emptied a list-valued key, which then has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved<'a> {
    /// The values of the key that count.
    Values(Values<'a>),
    /// The last value that emptied a list-valued key, which then has no value.
    Reset(&'a git::Occurrence),
}

/// The values of a key that has a value: one occurrence, or the occurrences of a list-valued key after its last reset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Values<'a> {
    /// The occurrence of a single-valued key.
    Single(&'a git::Occurrence),
    /// The occurrences of a list-valued key after its last reset, in load order; never empty.
    List(Listed<'a>),
}

/// The occurrences of a list-valued key after its last reset, in load order: at least one, by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed<'a> {
    /// All occurrences but the last, in load order.
    init: Vec<&'a git::Occurrence>,
    /// The last occurrence, to which the list is attributed.
    last: &'a git::Occurrence,
}

impl<'a> Listed<'a> {
    /// The list of `values`, or `None` when there are none.
    fn new(mut values: Vec<&'a git::Occurrence>) -> Option<Self> {
        let last = values.pop()?;

        Some(Self { init: values, last })
    }

    /// The occurrences of the list in load order.
    #[must_use = "the iterator is the only result of this method"]
    pub fn iter(&self) -> impl Iterator<Item = &'a git::Occurrence> {
        self.init.iter().copied().chain(iter::once(self.last))
    }
}

impl<'a> Values<'a> {
    /// The one occurrence a value is attributed to: the occurrence itself, or the last element of a list. A typed read
    /// converts this occurrence, and a group member takes its origin.
    #[must_use = "the attributed occurrence is the only result of this method"]
    pub const fn attributed(&self) -> &'a git::Occurrence {
        match self {
            Self::Single(occurrence) => occurrence,
            Self::List(listed) => listed.last,
        }
    }
}

/// The value of one key that has a value and the occurrence it is attributed to (`Values::attributed`).
struct Found<'a> {
    value: Value,
    from: &'a git::Occurrence,
}

/// Resolves the occurrences of a key to the one that counts, by the known row of the key (`None` for an unknown key).
///
/// A single-valued key has its last occurrence. A list-valued key has all its occurrences in load order; for a list
/// whose empty value resets it, the entries before the last empty value are dropped. A valueless occurrence is an
/// entry like any other. `None` when there is no occurrence; the caller decides which occurrences count, for example
/// by filtering out untrusted ones before resolving.
#[must_use = "the resolved occurrences are the only result of this lookup"]
pub fn resolve<'a, I>(known: Option<&KnownKey>, occurrences: I) -> Option<Resolved<'a>>
where
    I: Iterator<Item = &'a git::Occurrence>,
{
    match known.map_or(Multi::Single, |row| row.multi) {
        Multi::Single => occurrences
            .last()
            .map(|occurrence| Resolved::Values(Values::Single(occurrence))),
        Multi::List => {
            Listed::new(occurrences.collect()).map(|listed| Resolved::Values(Values::List(listed)))
        }
        Multi::ListWithReset => resolve_with_reset(occurrences),
    }
}

/// The untyped value of the values of a key, as `get` without `--type` returns it: a valueless occurrence is null, and
/// a known key is converted to its kind (bool, int or text). An element of a list is converted the same way.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigValueConversion`] for a value that its known kind cannot read, and
/// [`git::GixError::ConfigValueNotUtf8`] for a text value that is not UTF-8.
pub fn untyped_value(
    key: &git::ConfigKey,
    values: &Values<'_>,
    span: Span,
) -> Result<Value, git::GixError> {
    let kind = config_known::known(key).map_or(ValueKind::Text, |row| row.kind);

    match values {
        Values::Single(occurrence) => occurrence_value(key, occurrence, kind, span),
        Values::List(listed) => listed
            .iter()
            .map(|occurrence| occurrence_value(key, occurrence, kind, span))
            .collect::<Result<Vec<Value>, git::GixError>>()
            .map(|items| Value::list(items, span)),
    }
}

/// The value of `setting` read as `ty`, the way `gix config --type` reads it. A path reads only the trusted
/// occurrences, so an untrusted reset does not clear a trusted value. A list-valued key gives its last element. Null
/// when an optional path does not exist, or when no trusted occurrence is left for a path.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigKeyReset`] when a reset removed all the values, and the errors of the conversion of
/// the value that is read: [`git::GixError::ConfigValueMissing`], [`git::GixError::ConfigValueConversion`],
/// [`git::GixError::ConfigValueNotUtf8`], [`git::GixError::InstallDirUnavailable`] and
/// [`git::GixError::ConfigPathNotUtf8`].
pub fn typed_setting(
    setting: &git::Setting<'_>,
    ty: git::ConfigType,
    paths: &git::PathContext,
    span: Span,
) -> Result<Value, git::GixError> {
    let key = setting.key();
    let occurrences = setting.occurrences();
    let resolved = match ty {
        git::ConfigType::Scalar(_) => resolve(config_known::known(key), occurrences.iter()),
        git::ConfigType::Path => resolve(
            config_known::known(key),
            occurrences.iter().filter(|occurrence| occurrence.trusted),
        ),
    };

    match resolved {
        None => Ok(Value::nothing(span)),
        Some(resolved) => {
            let values = values_of(key, resolved)?;

            typed_value(key, values.attributed(), ty, paths, span)
        }
    }
}

/// Every setting of the configuration as the rows of the flat table `key`, `value`, `scope`, `file`: one row per
/// occurrence, sorted by key, and the occurrences of one key in load order.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigSubsectionNotUtf8`] for a subsection that is not UTF-8, and
/// [`git::GixError::ConfigValueNotUtf8`] or [`git::GixError::ConfigPathNotUtf8`] for a value or file that is not.
pub fn rows(tree: &git::ConfigTree<'_>, span: Span) -> Result<Vec<Value>, git::GixError> {
    tree.settings()
        .into_iter()
        .flat_map(|setting| {
            setting
                .occurrences()
                .iter()
                .map(move |occurrence| (setting.key(), occurrence))
        })
        .map(|(key, occurrence)| row(key, occurrence, span))
        .collect()
}

/// The row of one occurrence: its key as text, its value as text (null when valueless), its scope and its file.
fn row(
    key: &git::ConfigKey,
    occurrence: &git::Occurrence,
    span: Span,
) -> Result<Value, git::GixError> {
    let key_text = key.to_text().map_err(not_utf8(key, &occurrence.origin))?;
    let value = occurrence_value(key, occurrence, ValueKind::Text, span)?;
    let file = occurrence.origin.path.as_ref().map_or_else(
        || Ok(Value::nothing(span)),
        |path| path_text(key, occurrence, path.clone()).map(|text| Value::string(text, span)),
    )?;

    Ok(Value::record(
        record! {
            KEY_COLUMN => Value::string(key_text, span),
            VALUE_COLUMN => value,
            SCOPE_COLUMN => Value::string(occurrence.origin.scope().as_str(), span),
            FILE_COLUMN => file,
        },
        span,
    ))
}

/// The whole configuration as one nested record: a column per section, and a record per subsection inside it. A
/// variable, section or subsection with no value left after resets is omitted.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigSubsectionNotUtf8`] for a subsection that is not UTF-8 and has a column, and the
/// errors of [`untyped_value`].
pub fn everything(config: &git::Config, span: Span) -> Result<Value, git::GixError> {
    let tree = config.tree();
    let mut root = Record::new();

    for (name, section) in tree.sections() {
        let record = section_record(section, span)?;

        if !record.is_empty() {
            root.insert(name.as_str(), Value::record(record, span));
        }
    }

    Ok(Value::record(root, span))
}

/// The value a tree node holds: the record of a section or subsection, `None` when no column is left in it, or the
/// untyped value of a variable. A subsection node found by `ConfigTree::get` was named by a `&str`, so its name is
/// UTF-8 and is not decoded here.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigKeyReset`] when a reset removed all the values of a variable, and the errors of
/// [`everything`] for a record.
pub fn lookup_value(node: git::Node<'_>, span: Span) -> Result<Option<Value>, git::GixError> {
    match node {
        git::Node::Section(section) => Ok(Some(section_record(section, span)?)
            .filter(|record| !record.is_empty())
            .map(|record| Value::record(record, span))),
        git::Node::Subsection(subsection) => Ok(subsection_members(subsection, span)?
            .map(|members| Value::record(members.record, span))),
        git::Node::Setting(setting) => resolve(
            config_known::known(setting.key()),
            setting.occurrences().iter(),
        )
        .map(|resolved| {
            let values = values_of(setting.key(), resolved)?;

            untyped_value(setting.key(), &values, span)
        })
        .transpose(),
    }
}

/// The text of `subsection`, the subsection of `key`, wrapping a decoding failure with the `origin` the key was read
/// from.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigSubsectionNotUtf8`] when the subsection is not valid UTF-8.
pub fn subsection_text<'a>(
    key: &git::ConfigKey,
    subsection: &'a git::Subsection,
    origin: &Arc<git::Origin>,
) -> Result<&'a str, git::GixError> {
    subsection.to_text().map_err(not_utf8(key, origin))
}

/// The error for a subsection of `key` that is not UTF-8, naming where `key` was read.
fn not_utf8(
    key: &git::ConfigKey,
    origin: &Arc<git::Origin>,
) -> impl FnOnce(Utf8Error) -> git::GixError {
    let key = key.clone();
    let origin = Arc::clone(origin);

    move |source| git::GixError::ConfigSubsectionNotUtf8 {
        key,
        source,
        origin,
    }
}

/// The record of one section: its variables and, nested, its subsections that have a column left.
fn section_record(section: &git::SectionNode<'_>, span: Span) -> Result<Record, git::GixError> {
    let mut record = Record::new();

    for entry in section.entries() {
        match entry {
            git::Entry::Setting(setting) => {
                if let Some(found) = member(setting.key(), setting.occurrences(), span)? {
                    record.insert(setting.key().name().as_str(), found.value);
                }
            }
            git::Entry::Subsection(node) => {
                if let Some(members) = subsection_members(node, span)? {
                    let text = subsection_text(members.key, node.subsection(), &members.origin)?;

                    record.insert(text, Value::record(members.record, span));
                }
            }
        }
    }

    Ok(record)
}

/// The variables of a subsection that have a value left, with the first key and origin that carry one.
struct Members<'a> {
    record: Record,
    key: &'a git::ConfigKey,
    origin: Arc<git::Origin>,
}

/// The members of a subsection, or `None` when no variable has a value left.
fn subsection_members<'a>(
    node: &git::SubsectionNode<'a>,
    span: Span,
) -> Result<Option<Members<'a>>, git::GixError> {
    let mut record = Record::new();
    let mut first: Option<(&'a git::ConfigKey, Arc<git::Origin>)> = None;

    for setting in node.settings() {
        if let Some(found) = member(setting.key(), setting.occurrences(), span)? {
            record.insert(setting.key().name().as_str(), found.value);
            first.get_or_insert_with(|| (setting.key(), Arc::clone(&found.from.origin)));
        }
    }

    Ok(first.map(|(key, origin)| Members {
        record,
        key,
        origin,
    }))
}

/// The value of `key` as a group member, with the occurrence it is attributed to: `None` when a reset removed all the
/// values (an empty `occurrences` cannot come from `Config`, and ends the same way: no column).
fn member<'a>(
    key: &git::ConfigKey,
    occurrences: &'a [git::Occurrence],
    span: Span,
) -> Result<Option<Found<'a>>, git::GixError> {
    match resolve(config_known::known(key), occurrences.iter()) {
        None | Some(Resolved::Reset(_)) => Ok(None),
        Some(Resolved::Values(values)) => Ok(Some(Found {
            value: untyped_value(key, &values, span)?,
            from: values.attributed(),
        })),
    }
}

/// The resolution of a key whose empty value resets its list: a valueless occurrence is an entry, and an empty value
/// removes the entries before it.
fn resolve_with_reset<'a, I>(occurrences: I) -> Option<Resolved<'a>>
where
    I: Iterator<Item = &'a git::Occurrence>,
{
    let mut values: Vec<&'a git::Occurrence> = Vec::new();
    let mut reset: Option<&'a git::Occurrence> = None;

    for occurrence in occurrences {
        if is_empty_value(occurrence) {
            values.clear();
            reset = Some(occurrence);
        } else {
            values.push(occurrence);
        }
    }

    Listed::new(values)
        .map(|listed| Resolved::Values(Values::List(listed)))
        .or_else(|| reset.map(Resolved::Reset))
}

/// Whether an occurrence is an empty value (`name =`), which resets a list. A valueless occurrence is not.
const fn is_empty_value(occurrence: &git::Occurrence) -> bool {
    matches!(&occurrence.value, git::RawValue::Bytes(bytes) if bytes.is_empty())
}

/// The value of one occurrence without `--type`: null when it is valueless, otherwise converted to the kind of its key.
fn occurrence_value(
    key: &git::ConfigKey,
    occurrence: &git::Occurrence,
    kind: ValueKind,
    span: Span,
) -> Result<Value, git::GixError> {
    match &occurrence.value {
        git::RawValue::Valueless => Ok(Value::nothing(span)),
        git::RawValue::Bytes(_) => {
            git::convert(key, occurrence, scalar_type(kind)).map(|value| scalar_value(value, span))
        }
    }
}

/// The value of one occurrence read as `ty`, with the path rule for a path.
fn typed_value(
    key: &git::ConfigKey,
    occurrence: &git::Occurrence,
    ty: git::ConfigType,
    paths: &git::PathContext,
    span: Span,
) -> Result<Value, git::GixError> {
    match ty {
        git::ConfigType::Scalar(scalar) => {
            git::convert(key, occurrence, scalar).map(|value| scalar_value(value, span))
        }
        git::ConfigType::Path => git::convert_path(key, occurrence, paths)?.map_or_else(
            || Ok(Value::nothing(span)),
            |path| path_text(key, occurrence, path).map(|text| Value::string(text, span)),
        ),
    }
}

/// The Nushell value of a converted scalar.
fn scalar_value(value: git::ConfigValue, span: Span) -> Value {
    match value {
        git::ConfigValue::Bool(flag) => Value::bool(flag, span),
        git::ConfigValue::Int(number) => Value::int(number, span),
        git::ConfigValue::String(text) => Value::string(text, span),
    }
}

/// The text of a path, the one UTF-8 check of a path value.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigPathNotUtf8`] when the path is not valid UTF-8.
fn path_text(
    key: &git::ConfigKey,
    occurrence: &git::Occurrence,
    path: PathBuf,
) -> Result<String, git::GixError> {
    path.into_os_string().into_string().map_or_else(
        |_| {
            Err(git::GixError::ConfigPathNotUtf8 {
                key: key.clone(),
                origin: Arc::clone(&occurrence.origin),
            })
        },
        Ok,
    )
}

/// The scalar type an untyped read converts a known kind to; a path is read as its raw text unless `--type path`.
const fn scalar_type(kind: ValueKind) -> git::ScalarType {
    match kind {
        ValueKind::Bool => git::ScalarType::Bool,
        ValueKind::Int => git::ScalarType::Int,
        ValueKind::Text | ValueKind::Path => git::ScalarType::String,
    }
}

/// The values of a resolved key, or the error of a reset that removed all of them, naming the value that emptied it.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigKeyReset`] for a reset.
fn values_of<'a>(
    key: &git::ConfigKey,
    resolved: Resolved<'a>,
) -> Result<Values<'a>, git::GixError> {
    match resolved {
        Resolved::Values(values) => Ok(values),
        Resolved::Reset(occurrence) => Err(git::GixError::ConfigKeyReset {
            key: key.clone(),
            origin: Arc::clone(&occurrence.origin),
        }),
    }
}
