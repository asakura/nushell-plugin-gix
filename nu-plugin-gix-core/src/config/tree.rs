//! The configuration as nested nodes: sections, subsections and settings, each setting with every raw occurrence.

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;

use super::{Config, ConfigKey, Occurrence, SectionName, Subsection, VariableName};
use crate::error::GixError;

use std::collections::{BTreeMap, btree_map::Entry as MapEntry};

const KEY_SEPARATOR: char = '.';

/// A key the caller already split into parts; core never sees text or cell paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigPath {
    /// `section`: the whole section.
    Section(SectionName),
    /// `section.name`: the variable `name` of the section, or else its subsection `name`.
    Member {
        /// The section, lowercased.
        section: SectionName,
        /// The variable or subsection name as given.
        name: Box<str>,
    },
    /// `section.subsection.name`: a variable of a subsection.
    Variable {
        /// The section, lowercased.
        section: SectionName,
        /// The subsection, verbatim.
        subsection: Subsection,
        /// The variable name.
        name: VariableName,
    },
}

impl ConfigPath {
    /// The path of a whole section.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] with the section as its key when it is empty or contains `.`.
    pub fn section(section: &str) -> Result<Self, GixError> {
        let section = checked(section).ok_or_else(|| invalid_key(section))?;

        Ok(Self::Section(SectionName::from(section)))
    }

    /// The path of a variable of a section, or else of a subsection of it. `name` may contain `.`: it can then only
    /// name a subsection.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] with `<section>.<name>` as its key when the section is empty or
    /// contains `.`, or the name is empty.
    pub fn member(section: &str, name: &str) -> Result<Self, GixError> {
        let invalid = || invalid_key(&format!("{section}{KEY_SEPARATOR}{name}"));
        let section = checked(section).ok_or_else(invalid)?;
        let name = non_empty(name).ok_or_else(invalid)?;

        Ok(Self::Member {
            section: SectionName::from(section),
            name: name.into(),
        })
    }

    /// The path of a variable of a subsection; `subsection` is any text, even empty, kept verbatim.
    ///
    /// # Errors
    ///
    /// Returns [`GixError::InvalidConfigKey`] with `<section>.<subsection>.<name>` as its key when the section or the
    /// name is empty or contains `.`.
    pub fn variable(section: &str, subsection: &str, name: &str) -> Result<Self, GixError> {
        let invalid = || {
            invalid_key(&format!(
                "{section}{KEY_SEPARATOR}{subsection}{KEY_SEPARATOR}{name}"
            ))
        };
        let section = checked(section).ok_or_else(invalid)?;
        let name = checked(name).ok_or_else(invalid)?;

        Ok(Self::Variable {
            section: SectionName::from(section),
            subsection: Subsection::from(subsection),
            name: VariableName::from(name),
        })
    }
}

/// A section or variable name: non-empty and without `.`.
fn checked(part: &str) -> Option<&str> {
    non_empty(part).filter(|text| !text.contains(KEY_SEPARATOR))
}

/// The one rule for a key component: an empty section or variable name is invalid, as in git.
#[must_use = "the component is the only result"]
fn non_empty(part: &str) -> Option<&str> {
    (!part.is_empty()).then_some(part)
}

/// The error for a key part that is empty or contains `.`; `text` is the key as given.
fn invalid_key(text: &str) -> GixError {
    GixError::InvalidConfigKey {
        key: text.to_owned(),
    }
}

/// The configuration as nested nodes, borrowed from a [`Config`]. Sections ascend by name; the entries of a section
/// ascend by name bytes.
#[derive(Debug)]
pub struct ConfigTree<'a> {
    /// The sections by lowercased name.
    sections: BTreeMap<&'a SectionName, SectionNode<'a>>,
}

impl<'a> ConfigTree<'a> {
    /// Inserts every key of `config` into its section node; the result does not depend on the order of [`Config::iter`].
    pub(super) fn new(config: &'a Config) -> Self {
        let mut sections: BTreeMap<&SectionName, SectionNode<'_>> = BTreeMap::new();

        for (key, occurrences) in config.iter() {
            sections
                .entry(key.section())
                .or_default()
                .insert(Setting { key, occurrences });
        }

        Self { sections }
    }

    /// Every setting, hidden ones included, sorted by [`ConfigKey`].
    #[must_use = "the settings are the only result"]
    pub fn settings(&'a self) -> Vec<&'a Setting<'a>> {
        let mut settings: Vec<&'a Setting<'a>> = Vec::new();

        for section in self.sections.values() {
            for entry in section.entries.values() {
                match entry {
                    Entry::Setting(setting) => settings.push(setting),
                    Entry::Subsection(node) => settings.extend(node.settings.values()),
                }
            }

            settings.extend(&section.hidden);
        }

        settings.sort_by(|left, right| left.key.cmp(right.key));

        settings
    }

    /// The sections, ascending by name.
    #[must_use = "the iterator is the only result"]
    pub fn sections(&'a self) -> impl Iterator<Item = (&'a SectionName, &'a SectionNode<'a>)> {
        self.sections.iter().map(|(name, section)| (*name, section))
    }

    /// The node `path` names, or `None` when it names nothing.
    #[must_use = "the node is the only result of this lookup"]
    pub fn get(&'a self, path: &ConfigPath) -> Option<Node<'a>> {
        match path {
            ConfigPath::Section(section) => self.sections.get(section).map(Node::Section),
            ConfigPath::Member { section, name } => {
                let section = self.sections.get(section)?;

                section
                    .entries
                    .get(name.as_bytes())
                    .or_else(|| folded_setting(section, name))
                    .map(Node::from)
            }
            ConfigPath::Variable {
                section,
                subsection,
                name,
            } => {
                let section = self.sections.get(section)?;

                match section.entries.get(subsection.as_bytes())? {
                    Entry::Subsection(node) => node.settings.get(name).map(Node::Setting),
                    Entry::Setting(_) => None,
                }
            }
        }
    }
}

/// One section: one entry per name.
///
/// A variable and a subsection of one name cannot both be entries: the subsection wins as an entry, and the variable
/// is kept in `hidden` (never an entry, so never in a record or a key lookup, but still in [`ConfigTree::settings`]).
#[derive(Debug, Default)]
pub struct SectionNode<'a> {
    /// By name bytes: a variable's lowercase name, or the subsection bytes.
    entries: BTreeMap<&'a [u8], Entry<'a>>,
    /// Variables whose name is also the name of a subsection.
    hidden: Vec<Setting<'a>>,
}

impl<'a> SectionNode<'a> {
    /// The entries, ascending by name bytes.
    #[must_use = "the iterator is the only result"]
    pub fn entries(&'a self) -> impl Iterator<Item = &'a Entry<'a>> {
        self.entries.values()
    }

    /// Places `setting` in its subsection, or as a variable of the section; a subsection hides a variable of its name.
    fn insert(&mut self, setting: Setting<'a>) {
        let key = setting.key;

        match key.subsection() {
            None => match self.entries.entry(key.name().as_str().as_bytes()) {
                MapEntry::Vacant(vacant) => {
                    vacant.insert(Entry::Setting(setting));
                }
                // The name is held by a subsection, which wins (D5 L1). `Config` keys are unique, so no second variable
                // of this name arrives; if one did, the entry held would stay and the newcomer would be hidden, still
                // listed by `settings()`.
                MapEntry::Occupied(_) => self.hidden.push(setting),
            },
            Some(subsection) => match self.entries.entry(subsection.as_bytes()) {
                MapEntry::Vacant(vacant) => {
                    vacant.insert(Entry::Subsection(SubsectionNode::new(subsection, setting)));
                }
                MapEntry::Occupied(mut occupied) => match occupied.get_mut() {
                    Entry::Subsection(node) => {
                        node.settings.insert(key.name(), setting);
                    }
                    Entry::Setting(hidden) => {
                        self.hidden.push(*hidden);
                        occupied
                            .insert(Entry::Subsection(SubsectionNode::new(subsection, setting)));
                    }
                },
            },
        }
    }
}

/// An entry of a section.
#[derive(Debug)]
pub enum Entry<'a> {
    /// A variable of the section.
    Setting(Setting<'a>),
    /// A subsection of the section.
    Subsection(SubsectionNode<'a>),
}

/// A subsection with its variables.
#[derive(Debug)]
pub struct SubsectionNode<'a> {
    /// The subsection as read.
    subsection: &'a Subsection,
    /// The variables by lowercased name.
    settings: BTreeMap<&'a VariableName, Setting<'a>>,
}

impl<'a> SubsectionNode<'a> {
    /// A subsection holding `setting` only.
    fn new(subsection: &'a Subsection, setting: Setting<'a>) -> Self {
        Self {
            subsection,
            settings: BTreeMap::from([(setting.key.name(), setting)]),
        }
    }

    /// The subsection name.
    #[must_use = "the subsection is the only result"]
    pub const fn subsection(&self) -> &'a Subsection {
        self.subsection
    }

    /// The variables, ascending by name.
    #[must_use = "the iterator is the only result"]
    pub fn settings(&self) -> impl Iterator<Item = &Setting<'a>> {
        self.settings.values()
    }
}

/// One variable with every occurrence it has, in load order (never empty).
#[derive(Clone, Copy, Debug)]
pub struct Setting<'a> {
    /// The key of the variable.
    key: &'a ConfigKey,
    /// Its occurrences in load order.
    occurrences: &'a [Occurrence],
}

impl<'a> Setting<'a> {
    /// The key of the variable.
    #[must_use = "the key is the only result"]
    pub const fn key(&self) -> &'a ConfigKey {
        self.key
    }

    /// Every occurrence, in load order.
    #[must_use = "the occurrences are the only result"]
    pub const fn occurrences(&self) -> &'a [Occurrence] {
        self.occurrences
    }
}

/// What [`ConfigTree::get`] found.
#[derive(Clone, Copy, Debug)]
pub enum Node<'a> {
    /// A whole section.
    Section(&'a SectionNode<'a>),
    /// A subsection.
    Subsection(&'a SubsectionNode<'a>),
    /// A variable.
    Setting(&'a Setting<'a>),
}

impl<'a> From<&'a Entry<'a>> for Node<'a> {
    fn from(entry: &'a Entry<'a>) -> Self {
        match entry {
            Entry::Setting(setting) => Self::Setting(setting),
            Entry::Subsection(node) => Self::Subsection(node),
        }
    }
}

/// The variable of `section` named by the lowercased `name`, for a `name` that is not lowercase; a subsection is never
/// a match, and `None` at once when `name` is already lowercase (the exact lookup has covered it).
fn folded_setting<'a>(section: &'a SectionNode<'a>, name: &str) -> Option<&'a Entry<'a>> {
    let folded = VariableName::from(name);

    if folded.as_str() == name {
        return None;
    }

    section
        .entries
        .get(folded.as_str().as_bytes())
        .filter(|entry| matches!(entry, Entry::Setting(_)))
}
