//! Reading the layered git configuration: the raw occurrences of every variable, and the typed reading of one of them.
//!
//! The configuration is read once into a [`Config`] of [`Occurrence`]s, in load order, with each value kept as a
//! [`RawValue`] and with its [`Origin`] and trust. Nothing is selected there: the consumer decides which occurrence
//! counts. Each concern has its own submodule:
//!
//! - `store`: [`Config::read`] reads every occurrence and [`Config`] looks them up by [`ConfigKey`]; it decodes no
//!   subsection and no value. [`Config::from_occurrences`] builds a [`Config`] from given occurrences, for callers
//!   without a repository.
//! - `key`: [`ConfigKey`] and [`LookupPath`] parse the names a caller asks for, from text (the decomposition of gix)
//!   or from cell path members (`from_members`, never split or joined);
//!   [`GroupPath`] names a section or subsection.
//! - `origin`: [`Origin`] names where an occurrence comes from: its file path, or else its kind of source.
//! - `convert`: [`fn@convert`] reads one [`Occurrence`] as a [`ScalarType`] on request and [`fn@convert_path`] expands
//!   it as a path; neither selects an occurrence.
//!
//! Matching: section and variable names ignore case. A subsection matches exactly, as spelled in a quoted
//! `[section "subsection"]` header. A legacy `[section.subsection]` header is lowercased by `Config::read` (as git
//! does), so its subsection matches only a lowercase spelling.

mod convert;
mod key;
mod origin;
mod store;

pub use self::key::{ConfigKey, GroupPath, LookupPath, SectionName, Subsection, VariableName};
pub use self::{
    convert::{ConfigType, ConfigValue, PathContext, ScalarType, convert, convert_path},
    origin::Origin,
    store::{Config, Occurrence, RawValue},
};
