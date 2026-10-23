//! Reading the layered git configuration: the raw occurrences of every variable, and the typed reading of one of them.
//!
//! The configuration is read once into a [`Config`] of [`Occurrence`]s, in load order, with each value kept as a
//! [`RawValue`] and with its [`Origin`] and trust. Nothing is selected there: the consumer decides which occurrence
//! counts. Each concern has its own submodule:
//!
//! - `store`: [`Config::read`] reads every occurrence and [`Config`] looks them up by [`ConfigKey`]; it decodes no
//!   subsection and no value. [`Config::from_occurrences`] builds a [`Config`] from given occurrences, for callers
//!   without a repository.
//! - `key`: [`ConfigKey`] holds the already-split parts of a key (core parses no text);
//!   [`SectionHeader`] names a section or subsection.
//! - `tree`: [`Config::tree`] nests the occurrences as a [`ConfigTree`] of sections, subsections and [`Setting`]s, and
//!   [`ConfigTree::get`] finds the node a [`ConfigPath`] (a key already split into parts) names.
//! - `origin`: [`Origin`] names where an occurrence comes from: its file path, or else its kind of source; its
//!   [`Scope`] is the git scope word for that source.
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
mod tree;

pub use self::key::{ConfigKey, SectionHeader, SectionName, Subsection, VariableName};
pub use self::{
    convert::{ConfigType, ConfigValue, PathContext, ScalarType, convert, convert_path},
    origin::{Origin, Scope},
    store::{Config, Occurrence, RawValue},
    tree::{ConfigPath, ConfigTree, Entry, Node, SectionNode, Setting, SubsectionNode},
};
