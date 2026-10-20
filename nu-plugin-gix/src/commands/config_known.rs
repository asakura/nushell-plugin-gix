//! The known-key table: which configuration keys the plugin reads with a type and as lists.

use nu_plugin_gix_core as git;

use std::fmt;

/// Expands the rows `(section, name, rule, kind, multi)` once into `TABLE` and `known`.
///
/// Two rows sharing (section, name) become an unreachable match arm in `known`, which is a compile error.
macro_rules! known_keys {
    ($(($section:literal, $name:literal, $rule:expr, $kind:expr, $multi:expr)),+ $(,)?) => {
        /// Git has no schema; this curated list is the plugin's read policy (user decisions U7, U10).
        /// Read by `known` and by the help text of `gix config get`.
        pub const TABLE: &[KnownKey] = &[
            $(KnownKey {
                section: $section,
                subsection: $rule,
                name: $name,
                kind: $kind,
                multi: $multi,
            }),+
        ];

        /// Returns the table row for `key`, or `None` when the key is not known.
        ///
        /// The match on (section, name) admits one row per pair, so no key can match two rows.
        #[must_use = "the row decides how the key is read; discarding it skips the lookup"]
        pub fn known(key: &git::ConfigKey) -> Option<&'static KnownKey> {
            let row: &'static KnownKey = match (key.section().as_str(), key.name().as_str()) {
                $(($section, $name) => &KnownKey {
                    section: $section,
                    subsection: $rule,
                    name: $name,
                    kind: $kind,
                    multi: $multi,
                },)+
                _ => return None,
            };

            subsection_matches(row.subsection, key.subsection().is_some()).then_some(row)
        }
    };
}

known_keys! {
    ("blame", "ignorerevsfile", SubsectionRule::Absent, ValueKind::Path, Multi::ListWithReset),
    ("branch", "merge", SubsectionRule::Present("name"), ValueKind::Text, Multi::List),
    ("commit", "gpgsign", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("core", "bare", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("core", "bigfilethreshold", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("core", "compression", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("core", "deltabasecachelimit", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("core", "filemode", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("core", "gitproxy", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("core", "ignorecase", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("core", "precomposeunicode", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("core", "symlinks", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("credential", "helper", SubsectionRule::Either("url"), ValueKind::Text, Multi::ListWithReset),
    ("fetch", "parallel", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("fetch", "prune", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("fetch", "prunetags", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("format", "notes", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("gc", "auto", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("hook", "event", SubsectionRule::Present("friendly-name"), ValueKind::Text, Multi::ListWithReset),
    ("http", "extraheader", SubsectionRule::Either("url"), ValueKind::Text, Multi::ListWithReset),
    ("http", "curloptresolve", SubsectionRule::Either("url"), ValueKind::Text, Multi::ListWithReset),
    ("http", "lowspeedlimit", SubsectionRule::Either("url"), ValueKind::Int, Multi::Single),
    ("http", "lowspeedtime", SubsectionRule::Either("url"), ValueKind::Int, Multi::Single),
    ("http", "postbuffer", SubsectionRule::Either("url"), ValueKind::Int, Multi::Single),
    ("http", "sslverify", SubsectionRule::Either("url"), ValueKind::Bool, Multi::Single),
    ("include", "path", SubsectionRule::Absent, ValueKind::Path, Multi::List),
    ("includeif", "path", SubsectionRule::Present("condition"), ValueKind::Path, Multi::List),
    ("maintenance", "repo", SubsectionRule::Absent, ValueKind::Path, Multi::List),
    ("merge", "suppressdest", SubsectionRule::Absent, ValueKind::Text, Multi::ListWithReset),
    ("notes", "displayref", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("pack", "threads", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("push", "autosetupremote", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("push", "followtags", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("push", "pushoption", SubsectionRule::Absent, ValueKind::Text, Multi::ListWithReset),
    ("rebase", "autostash", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("rebase", "autosquash", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("receive", "procreceiverefs", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("receive", "hiderefs", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("remote", "url", SubsectionRule::Present("name"), ValueKind::Text, Multi::ListWithReset),
    ("remote", "pushurl", SubsectionRule::Present("name"), ValueKind::Text, Multi::ListWithReset),
    ("remote", "serveroption", SubsectionRule::Present("name"), ValueKind::Text, Multi::ListWithReset),
    ("remote", "negotiationinclude", SubsectionRule::Present("name"), ValueKind::Text, Multi::ListWithReset),
    ("remote", "negotiationrestrict", SubsectionRule::Present("name"), ValueKind::Text, Multi::ListWithReset),
    ("remote", "fetch", SubsectionRule::Present("name"), ValueKind::Text, Multi::List),
    ("remote", "push", SubsectionRule::Present("name"), ValueKind::Text, Multi::List),
    ("safe", "directory", SubsectionRule::Absent, ValueKind::Path, Multi::ListWithReset),
    ("submodule", "active", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("submodule", "fetchjobs", SubsectionRule::Absent, ValueKind::Int, Multi::Single),
    ("tag", "gpgsign", SubsectionRule::Absent, ValueKind::Bool, Multi::Single),
    ("transfer", "hiderefs", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("uploadpack", "hiderefs", SubsectionRule::Absent, ValueKind::Text, Multi::List),
    ("url", "insteadof", SubsectionRule::Present("base"), ValueKind::Text, Multi::List),
    ("url", "pushinsteadof", SubsectionRule::Present("base"), ValueKind::Text, Multi::List),
    ("versionsort", "suffix", SubsectionRule::Absent, ValueKind::Text, Multi::List),
}

/// The value type a known key is read as when the request names no type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// Read as a boolean.
    Bool,
    /// Read as an integer.
    Int,
    /// Read as text.
    Text,
    /// A key git reads through `git_config_pathname` or `interpolate_path` (tilde expansion); an untyped read still returns the raw text.
    Path,
}

/// How the occurrences of a known key combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Multi {
    /// One value; the last occurrence wins.
    Single,
    /// Every occurrence is a list entry, in load order.
    List,
    /// A list in which an empty value clears the entries before it.
    ListWithReset,
}

/// Whether a known key takes a subsection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsectionRule {
    /// Only `section.name`.
    Absent,
    /// Only `section.<placeholder>.name`; the payload is git's placeholder name.
    Present(&'static str),
    /// Both forms; the payload is git's placeholder name.
    Either(&'static str),
}

/// One row of the known-key table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KnownKey {
    /// The lowercase section name.
    pub section: &'static str,
    /// Whether the key takes a subsection.
    pub subsection: SubsectionRule,
    /// The lowercase variable name.
    pub name: &'static str,
    /// The type the key is read as when the request names none.
    pub kind: ValueKind,
    /// How the occurrences of the key combine.
    pub multi: Multi,
}

impl fmt::Display for KnownKey {
    /// `section.name`, `section.<p>.name` or `section.[<p>.]name`, with `p` the rule's placeholder.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.subsection {
            SubsectionRule::Absent => write!(f, "{}.{}", self.section, self.name),
            SubsectionRule::Present(placeholder) => {
                write!(f, "{}.<{placeholder}>.{}", self.section, self.name)
            }
            SubsectionRule::Either(placeholder) => {
                write!(f, "{}.[<{placeholder}>.]{}", self.section, self.name)
            }
        }
    }
}

/// Whether a key with or without a subsection fits `rule`.
#[must_use = "the verdict decides whether the row applies; discarding it skips the check"]
const fn subsection_matches(rule: SubsectionRule, has_subsection: bool) -> bool {
    match (rule, has_subsection) {
        (SubsectionRule::Either(_), _)
        | (SubsectionRule::Absent, false)
        | (SubsectionRule::Present(_), true) => true,
        (SubsectionRule::Absent, true) | (SubsectionRule::Present(_), false) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rstest::rstest;

    use std::assert_matches;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Git(#[from] Box<git::GixError>),

        #[error("Test failure: {0}")]
        Failure(String),
    }

    impl From<git::GixError> for TestError {
        fn from(error: git::GixError) -> Self {
            Self::Git(Box::new(error))
        }
    }

    mod known {
        use super::*;

        fn multi_of(text: &str) -> Result<Option<Multi>, TestError> {
            Ok(known(&git::ConfigKey::parse(text)?).map(|row| row.multi))
        }

        fn kind_of(text: &str) -> Result<Option<ValueKind>, TestError> {
            Ok(known(&git::ConfigKey::parse(text)?).map(|row| row.kind))
        }

        mod multi {
            use super::*;

            #[rstest]
            #[case::blame_ignorerevsfile("blame.ignorerevsfile")]
            #[case::credential_helper("credential.helper")]
            #[case::credential_url_helper("credential.https://example.com.helper")]
            #[case::http_extraheader("http.extraheader")]
            #[case::http_curloptresolve("http.curloptresolve")]
            #[case::hook_event("hook.pre-push.event")]
            #[case::merge_suppressdest("merge.suppressdest")]
            #[case::push_pushoption("push.pushoption")]
            #[case::remote_url("remote.origin.url")]
            #[case::remote_pushurl("remote.origin.pushurl")]
            #[case::remote_serveroption("remote.origin.serveroption")]
            #[case::remote_negotiationinclude("remote.origin.negotiationinclude")]
            #[case::remote_negotiationrestrict("remote.origin.negotiationrestrict")]
            #[case::safe_directory("safe.directory")]
            fn reset_rows_are_list_with_reset(#[case] text: &str) -> Result<(), TestError> {
                assert_eq!(multi_of(text)?, Some(Multi::ListWithReset));

                Ok(())
            }

            #[rstest]
            #[case::receive_procreceiverefs("receive.procreceiverefs")]
            #[case::versionsort_suffix("versionsort.suffix")]
            #[case::core_gitproxy("core.gitproxy")]
            #[case::branch_merge("branch.main.merge")]
            #[case::format_notes("format.notes")]
            #[case::notes_displayref("notes.displayref")]
            #[case::transfer_hiderefs("transfer.hiderefs")]
            #[case::uploadpack_hiderefs("uploadpack.hiderefs")]
            #[case::receive_hiderefs("receive.hiderefs")]
            #[case::submodule_active("submodule.active")]
            #[case::remote_fetch("remote.origin.fetch")]
            #[case::remote_push("remote.origin.push")]
            #[case::include_path("include.path")]
            #[case::includeif_path("includeif.gitdir:/repo/.path")]
            #[case::url_insteadof("url.https://example.com/.insteadof")]
            #[case::url_pushinsteadof("url.https://example.com/.pushinsteadof")]
            #[case::maintenance_repo("maintenance.repo")]
            fn list_rows_are_list(#[case] text: &str) -> Result<(), TestError> {
                assert_eq!(multi_of(text)?, Some(Multi::List));

                Ok(())
            }
        }

        mod kind {
            use super::*;

            #[rstest]
            #[case::core_bare("core.bare", ValueKind::Bool)]
            #[case::core_filemode("core.filemode", ValueKind::Bool)]
            #[case::core_ignorecase("core.ignorecase", ValueKind::Bool)]
            #[case::core_symlinks("core.symlinks", ValueKind::Bool)]
            #[case::core_precomposeunicode("core.precomposeunicode", ValueKind::Bool)]
            #[case::commit_gpgsign("commit.gpgsign", ValueKind::Bool)]
            #[case::tag_gpgsign("tag.gpgsign", ValueKind::Bool)]
            #[case::fetch_prune("fetch.prune", ValueKind::Bool)]
            #[case::fetch_prunetags("fetch.prunetags", ValueKind::Bool)]
            #[case::push_autosetupremote("push.autosetupremote", ValueKind::Bool)]
            #[case::push_followtags("push.followtags", ValueKind::Bool)]
            #[case::rebase_autostash("rebase.autostash", ValueKind::Bool)]
            #[case::rebase_autosquash("rebase.autosquash", ValueKind::Bool)]
            #[case::http_sslverify("http.sslverify", ValueKind::Bool)]
            #[case::gc_auto("gc.auto", ValueKind::Int)]
            #[case::core_compression("core.compression", ValueKind::Int)]
            #[case::core_bigfilethreshold("core.bigfilethreshold", ValueKind::Int)]
            #[case::http_postbuffer("http.postbuffer", ValueKind::Int)]
            #[case::fetch_parallel("fetch.parallel", ValueKind::Int)]
            #[case::submodule_fetchjobs("submodule.fetchjobs", ValueKind::Int)]
            #[case::pack_threads("pack.threads", ValueKind::Int)]
            #[case::http_lowspeedlimit("http.lowspeedlimit", ValueKind::Int)]
            #[case::http_lowspeedtime("http.lowspeedtime", ValueKind::Int)]
            #[case::core_deltabasecachelimit("core.deltabasecachelimit", ValueKind::Int)]
            fn scalar_rows_have_their_kind(
                #[case] text: &str,
                #[case] expected: ValueKind,
            ) -> Result<(), TestError> {
                assert_eq!(kind_of(text)?, Some(expected));

                Ok(())
            }
        }

        mod subsection {
            use super::*;

            #[rstest]
            #[case::user_name("user.name")]
            #[case::remote_pushurl2("remote.origin.pushurl2")]
            #[case::remote_without_subsection("remote.fetch")]
            #[case::include_with_subsection("include.x.path")]
            #[case::includeif_without_subsection("includeif.path")]
            #[case::fetch_negotiationtip_is_not_a_git_variable("fetch.negotiationtip")]
            fn unknown_shape_is_none(#[case] text: &str) -> Result<(), TestError> {
                assert_eq!(known(&git::ConfigKey::parse(text)?), None);

                Ok(())
            }

            #[rstest]
            #[case::credential_without_url("credential.helper")]
            #[case::credential_with_url("credential.https://example.com.helper")]
            #[case::http_extraheader_without_url("http.extraheader")]
            #[case::http_extraheader_with_url("http.https://example.com.extraheader")]
            #[case::http_curloptresolve_without_url("http.curloptresolve")]
            #[case::http_curloptresolve_with_url("http.https://example.com.curloptresolve")]
            #[case::http_sslverify_without_url("http.sslverify")]
            #[case::http_sslverify_with_url("http.https://example.com.sslverify")]
            #[case::http_postbuffer_without_url("http.postbuffer")]
            #[case::http_postbuffer_with_url("http.https://example.com.postbuffer")]
            #[case::http_lowspeedlimit_without_url("http.lowspeedlimit")]
            #[case::http_lowspeedlimit_with_url("http.https://example.com.lowspeedlimit")]
            #[case::http_lowspeedtime_without_url("http.lowspeedtime")]
            #[case::http_lowspeedtime_with_url("http.https://example.com.lowspeedtime")]
            fn either_rule_accepts_both_forms(#[case] text: &str) -> Result<(), TestError> {
                let key = git::ConfigKey::parse(text)?;

                assert_matches!(
                    known(&key).map(|row| row.subsection),
                    Some(SubsectionRule::Either(_))
                );

                Ok(())
            }
        }
    }

    mod known_key {
        use super::*;

        #[rstest]
        #[case::absent("core.bare", "core.bare")]
        #[case::remote("remote.origin.url", "remote.<name>.url")]
        #[case::includeif("includeif.gitdir:/repo/.path", "includeif.<condition>.path")]
        #[case::url("url.https://example.com/.insteadof", "url.<base>.insteadof")]
        #[case::credential("credential.helper", "credential.[<url>.]helper")]
        #[case::http("http.sslverify", "http.[<url>.]sslverify")]
        #[case::branch("branch.main.merge", "branch.<name>.merge")]
        #[case::hook("hook.pre-push.event", "hook.<friendly-name>.event")]
        fn display_uses_git_placeholders(
            #[case] text: &str,
            #[case] expected: &str,
        ) -> Result<(), TestError> {
            let key = git::ConfigKey::parse(text)?;
            let row = known(&key).ok_or_else(|| TestError::Failure("unknown key".into()))?;

            assert_eq!(row.to_string(), expected);

            Ok(())
        }
    }

    mod table {
        use super::*;

        #[rstest]
        fn names_are_lowercase() {
            for row in TABLE {
                assert_eq!(row.section, row.section.to_ascii_lowercase());
                assert_eq!(row.name, row.name.to_ascii_lowercase());
            }
        }

        #[rstest]
        fn known_agrees_with_table() -> Result<(), TestError> {
            for row in TABLE {
                let mut texts = Vec::new();

                match row.subsection {
                    SubsectionRule::Absent => texts.push([row.section, ".", row.name].concat()),
                    SubsectionRule::Present(_) => {
                        texts.push([row.section, ".sub.", row.name].concat());
                    }
                    SubsectionRule::Either(_) => {
                        texts.push([row.section, ".sub.", row.name].concat());
                        texts.push([row.section, ".", row.name].concat());
                    }
                }

                for text in texts {
                    let key = git::ConfigKey::parse(&text)?;

                    assert_eq!(known(&key), Some(row), "{text}");
                }
            }

            Ok(())
        }

        #[rstest]
        fn path_keys() {
            let mut paths: Vec<String> = TABLE
                .iter()
                .filter(|row| row.kind == ValueKind::Path)
                .map(ToString::to_string)
                .collect();
            paths.sort();

            assert_eq!(
                paths,
                [
                    "blame.ignorerevsfile",
                    "include.path",
                    "includeif.<condition>.path",
                    "maintenance.repo",
                    "safe.directory",
                ]
            );
        }
    }
}
