//! Reads the configuration of a repository as Nushell values: the occurrences a key resolves to, their typed or untyped
//! values, and the records of sections and subsections.

#[cfg(test)]
#[path = "config_read_tests.rs"]
mod tests;

use super::config_known::{self, KnownKey, Multi, ValueKind};

use nu_plugin_gix_core as git;
use nu_protocol::{Record, Span, Value};

use std::{
    collections::{BTreeMap, btree_map::Entry},
    iter,
    path::PathBuf,
    sync::Arc,
};

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

/// A section or subsection read as a record, or why there is none.
#[derive(Clone, Debug, PartialEq)]
pub enum Group {
    /// No key of the group is set.
    Missing,
    /// Keys of the group are set, but resets removed all their values.
    AllReset,
    /// The values of the group, with each subsection nested as a record.
    Record(Record),
}

/// The value of one key that has a value and the occurrence it is attributed to (`Values::attributed`).
struct Found<'a> {
    value: Value,
    from: &'a git::Occurrence,
}

/// The variables of one subsection by name, the columns of its record.
#[derive(Debug, Default)]
struct Members(BTreeMap<String, Value>);

/// A subsection of a section with its variables, and the origin of its first emitted variable: a variable that a reset
/// removed is not emitted, so it does not count.
#[derive(Debug)]
struct Subsection {
    members: Members,
    origin: Arc<git::Origin>,
}

/// A column of a section record: a variable, or a subsection with its variables.
#[derive(Debug)]
enum Column {
    Variable {
        name: git::VariableName,
        value: Value,
        origin: Arc<git::Origin>,
    },
    Subsection(Subsection),
}

/// The columns of one section by their text. A variable and a subsection of one text are two kinds of one column: one
/// map, so they cannot both exist, whatever the order they are inserted in. The map order is the column order.
#[derive(Debug, Default)]
struct Nest {
    columns: BTreeMap<String, Column>,
}

impl Members {
    /// Adds the variable `name` with `value`.
    fn insert(&mut self, name: String, value: Value) {
        self.0.insert(name, value);
    }

    /// The record of the variables.
    fn into_record(self) -> Record {
        self.0.into_iter().collect()
    }
}

impl Nest {
    /// Adds the value of `key` from its occurrences, unless no occurrence counts or a reset removed all of them.
    fn insert_key(
        &mut self,
        key: &git::ConfigKey,
        occurrences: &[git::Occurrence],
        span: Span,
    ) -> Result<(), git::GixError> {
        match member(key, occurrences, span)? {
            None => Ok(()),
            Some(found) => match &key.subsection() {
                None => self.insert_variable(key, found),
                Some(subsection) => {
                    let text = subsection.to_text().map_err(|source| {
                        git::GixError::ConfigSubsectionNotUtf8 {
                            key: key.clone(),
                            source,
                            origin: Arc::clone(&found.from.origin),
                        }
                    })?;

                    self.insert_subsection_member(key, text, found)
                }
            },
        }
    }

    /// Adds `found` as a variable of the section, unless a subsection of its name is already a column. A key occurs
    /// once in `Config`; a variable of the same name would be replaced by the later one.
    fn insert_variable(
        &mut self,
        key: &git::ConfigKey,
        found: Found<'_>,
    ) -> Result<(), git::GixError> {
        let column = Column::Variable {
            name: key.name().clone(),
            value: found.value,
            origin: Arc::clone(&found.from.origin),
        };

        match self.columns.entry(key.name().as_str().to_owned()) {
            Entry::Vacant(slot) => {
                slot.insert(column);
            }
            Entry::Occupied(mut taken) => {
                if let Column::Subsection(subsection) = taken.get() {
                    return Err(name_clash(
                        key.section(),
                        key.name(),
                        Arc::clone(&found.from.origin),
                        Arc::clone(&subsection.origin),
                    ));
                }

                taken.insert(column);
            }
        }

        Ok(())
    }

    /// Adds `found` as a variable of the subsection `subsection` of the section, unless a variable of its name is
    /// already a column. The subsection's origin is that of its first member.
    fn insert_subsection_member(
        &mut self,
        key: &git::ConfigKey,
        subsection: &str,
        found: Found<'_>,
    ) -> Result<(), git::GixError> {
        match self.columns.entry(subsection.to_owned()) {
            Entry::Vacant(slot) => {
                let mut members = Members::default();
                members.insert(key.name().as_str().to_owned(), found.value);
                slot.insert(Column::Subsection(Subsection {
                    members,
                    origin: Arc::clone(&found.from.origin),
                }));

                Ok(())
            }
            Entry::Occupied(mut taken) => match taken.get_mut() {
                Column::Variable {
                    name,
                    origin,
                    value: _value,
                } => Err(name_clash(
                    key.section(),
                    name,
                    Arc::clone(origin),
                    Arc::clone(&found.from.origin),
                )),
                Column::Subsection(existing) => {
                    existing
                        .members
                        .insert(key.name().as_str().to_owned(), found.value);

                    Ok(())
                }
            },
        }
    }

    /// The record of the variables and subsections, with the columns in alphabetical order.
    fn into_record(self, span: Span) -> Record {
        self.columns
            .into_iter()
            .map(|(text, column)| {
                let value = match column {
                    Column::Variable {
                        value,
                        name: _name,
                        origin: _origin,
                    } => value,
                    Column::Subsection(subsection) => {
                        Value::record(subsection.members.into_record(), span)
                    }
                };

                (text, value)
            })
            .collect()
    }
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

/// The untyped value of `key`, or `None` when the key is unset.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigKeyReset`] when a reset removed all the values, and the errors of [`untyped_value`]
/// for the value the key resolves to.
pub fn read_variable(
    config: &git::Config,
    key: &git::ConfigKey,
    span: Span,
) -> Result<Option<Value>, git::GixError> {
    match resolve(config_known::known(key), config.occurrences(key).iter()) {
        None => Ok(None),
        Some(resolved) => {
            let values = values_of(key, resolved)?;

            untyped_value(key, &values, span).map(Some)
        }
    }
}

/// The value of `key` read as `ty`, the way `get --type` reads it. A path reads only the trusted occurrences, so an
/// untrusted reset does not clear a trusted value. A list-valued key gives its last element. Null when the key is unset,
/// or when an optional path does not exist.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigKeyReset`] when a reset removed all the values, and the errors of the conversion of
/// the value that is read: [`git::GixError::ConfigValueMissing`], [`git::GixError::ConfigValueConversion`],
/// [`git::GixError::ConfigValueNotUtf8`], [`git::GixError::InstallDirUnavailable`] and
/// [`git::GixError::ConfigPathNotUtf8`].
pub fn read_typed(
    config: &git::Config,
    key: &git::ConfigKey,
    ty: git::ConfigType,
    paths: &git::PathContext,
    span: Span,
) -> Result<Value, git::GixError> {
    let occurrences = config.occurrences(key);
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

/// The section `section` as a record of its variables and subsections, or why it has none. Keys that a reset removed
/// are left out.
///
/// # Errors
///
/// Returns [`git::GixError::ConfigNameClash`] when a variable and a subsection of the section have the same name,
/// [`git::GixError::ConfigSubsectionNotUtf8`] for a subsection of the section that is not UTF-8, and the errors of
/// [`untyped_value`].
pub fn read_section(
    config: &git::Config,
    section: &git::SectionName,
    span: Span,
) -> Result<Group, git::GixError> {
    let mut nest = Nest::default();
    let mut seen = false;

    for (key, occurrences) in config.iter().filter(|(key, _)| key.section() == section) {
        seen = true;
        nest.insert_key(key, occurrences, span)?;
    }

    Ok(group_of(seen, nest.into_record(span)))
}

/// The subsection `subsection` of `section` as a record of its variables, or why it has none. Keys that a reset
/// removed are left out.
///
/// # Errors
///
/// Returns the errors of [`untyped_value`] for a value of the subsection.
pub fn read_subsection(
    config: &git::Config,
    section: &git::SectionName,
    subsection: &git::Subsection,
    span: Span,
) -> Result<Group, git::GixError> {
    let mut members = Members::default();
    let mut seen = false;

    for (key, occurrences) in config
        .iter()
        .filter(|(key, _)| key.section() == section && key.subsection() == Some(subsection))
    {
        seen = true;

        if let Some(found) = member(key, occurrences, span)? {
            members.insert(key.name().as_str().to_owned(), found.value);
        }
    }

    Ok(group_of(seen, members.into_record()))
}

/// Every section of the configuration as a record of its variables and subsections, with the sections of only reset
/// keys left out.
///
/// # Errors
///
/// Returns the errors of [`read_section`] for any section.
pub fn read_all(config: &git::Config, span: Span) -> Result<Value, git::GixError> {
    let mut sections: BTreeMap<&str, Nest> = BTreeMap::new();

    for (key, occurrences) in config.iter() {
        sections
            .entry(key.section().as_str())
            .or_default()
            .insert_key(key, occurrences, span)?;
    }

    let mut root = Record::new();

    for (section, nest) in sections {
        let record = nest.into_record(span);

        if !record.is_empty() {
            root.insert(section, Value::record(record, span));
        }
    }

    Ok(Value::record(root, span))
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

/// The group that `seen` keys with the record `record` make: missing when no key was seen, all reset when the
/// record is empty.
fn group_of(seen: bool, record: Record) -> Group {
    match (seen, record.is_empty()) {
        (false, _) => Group::Missing,
        (true, true) => Group::AllReset,
        (true, false) => Group::Record(record),
    }
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

/// The error for a variable and a subsection of one section that have the same text. `name` is the variable's own
/// name, which equals the subsection text.
fn name_clash(
    section: &git::SectionName,
    name: &git::VariableName,
    variable_origin: Arc<git::Origin>,
    subsection_origin: Arc<git::Origin>,
) -> git::GixError {
    git::GixError::ConfigNameClash {
        section: section.clone(),
        name: name.clone(),
        variable_origin,
        subsection_origin,
    }
}
