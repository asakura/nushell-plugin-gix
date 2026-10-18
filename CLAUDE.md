````toml
# nu_plugin_gix: treat this block as binding compiler policy
# (lint levels live in Cargo.toml [lints.*]; clippy thresholds in clippy.toml)

[deny]
lints = ["all","pedantic","nursery","cargo","restriction", "unwrap_used","expect_used","panic","todo","unimplemented","dbg", "arithmetic_side_effects","indexing_slicing","str_to_string","absolute_paths", "missing_debug_implementations","missing_copy_implementations","missing_docs", "rustdoc::all","print_stdout","print_stderr","format_collect", "allow_attributes","allow_attributes_without_reason","unfulfilled_lint_expectations"]
forbid  = ["unsafe_code"]
# allow_attributes is also deny — #[allow(...)] is itself a hard error
suppress = "#[expect(clippy::foo, reason='…')]"
test_suppress = "#![cfg_attr(test, expect(clippy::foo, reason='…'))] at top of the crate root"
# restriction group has intentionally contradictory lints; the subset allowed in
# Cargo.toml is policy — never suppress these with #[expect]:
restriction_dead     = ["implicit_return", "single_char_lifetime_names", "question_mark_used", "module_name_repetitions","missing_inline_in_public_items", "missing_trait_methods", "pub_use", "exhaustive_structs", "exhaustive_enums", "missing_docs_in_private_items", "single_call_fn","wildcard_enum_match_arm", "shadow_reuse", "std_instead_of_alloc", "std_instead_of_core", "min_ident_chars", "iter_over_hash_type","inline_modules", "pub_with_shorthand", "separated_literal_suffix", "assertions_on_result_states", "pattern_type_mismatch","arbitrary_source_item_ordering", "mod_module_files"]
restriction_dead_why = "conflict with project style, idiomatic Rust, or each other"

[types]
strings   = "no .to_string() (str_to_string denied) — use .to_owned()/.into(); no format!() for plain string construction"
propagate = "? to an appropriate Result — no .unwrap()/.expect() anywhere incl. tests; unreachable→prove with types/exhaustiveness"
math      = "saturating_*|checked_*|wrapping_* only; no bare +-*/"
slices    = ".get(n)? or iter/windows/chunks; no [n]"
ordering  = "BTreeMap when key order matters; LazyLock for expensive statics"
enums     = "illegal-states-unrepresentable"
zst       = "derive(Debug,Clone,Copy,Default); #[must_use] on every pure constructor/builder"
must_use  = "#[must_use] on every pure fn/method whose result must not be discarded; every #[must_use] MUST carry a reason string, no bare #[must_use]"
copy      = "every pub type that can be Copy (no heap, no Drop) must derive it; when unsure attempt #[derive(Copy,Clone)] — compiler decides"
const     = "named const for every magic literal (string sentinel, numeric limit, command name, flag name); group before first use"
generics  = "free fns + inherent methods: fn f<S>(x: S) where S: Bound — no impl Trait in params, no inline bounds; trait methods: impl Trait in params OK"

[errors]
lib         = "thiserror; one Error type per module boundary (git::GixError, commands::CommandError), re-exported at the module root; #[from] foreign errors"
nushell     = "convert to nu_protocol::LabeledError only at the PluginCommand::run boundary (with a span pointing at the offending argument/input); never deeper, and never inside src/git (see [boundary])"
msgs        = "#[error] human-readable, not Debug"
context     = "no anyhow/miette/eyre; context lives in typed variant fields (#[source] inner + path/name fields); never .context() chains"

[files]
order   = "cfg_attr(crate-root-only) → //! → use → const → types(public-first) → impl X → impl Display → fns → mod tests"
imports = "3 blank-separated groups in order: (1) crate-internal (2) external crates (3) std"
lib     = "explicit pub use (no wildcard); no redundant /// above pub mod when //! exists inside"
deps    = "default-features=false unless a feature is needed; justify every new dependency; extend existing ones before adding"
gix     = "enable only the gix features actually used (Cargo.toml lists them explicitly)"

[tests]
structure = "nested submods per type/fn; each opens use super::*; never flat"
attrs     = "#[rstest] always; #[case::snake_name] mandatory (bare #[case] forbidden); redundant_test_prefix denied"
sig       = "Result<(),TestError>+?; no unwrap in test body; assert_matches!; 1 behavior/test"
testerror = "thiserror per mod tests; always include Failure(String) variant; #[from] for every foreign error; never reuse domain errors"
data      = "file fixtures → tests/fixtures/<scenario-name>/, committed; name describes the scenario; inline for <10 lines"
isolation = "filesystem mutations → tempfile::TempDir via rstest #[fixture]; git repos under test are created in a TempDir, never the working repo; env-var mutations → serial + restore in fixture Drop"

[docs]
all      = "//! every module; one sentence for leaf modules (error.rs, types.rs)"
public   = "/// all public items; private helpers with non-obvious invariants also get ///"
style    = "omit comments that restate the name"
doctest  = "rustdoc::all is denied — examples must compile; hide setup with # prefix; ```rust,no_run when a live repo/runtime is needed; never ```rust,ignore"
# NOTE: this is a [[bin]] crate — cargo does not run its doc examples. Logic that
# needs tested examples belongs in a lib target.

[arch]
shape    = "Nushell plugin: binary named nu_plugin_* speaking the plugin protocol over stdio (MsgPackSerializer); registered in nushell with `plugin add`"
stdio    = "stdout/stderr carry the protocol — never print (print_stdout/print_stderr denied); report problems as LabeledError"
versions = "nu-plugin/nu-protocol pinned to the Nushell release the plugin targets (currently 0.112.2); gix 0.89 — bump deliberately, together"
commands = "one PluginCommand per Nushell command; signature(), description and examples are user-facing — keep them accurate"

[boundary]
# two sides, one direction of dependency: Nushell side -> git side, never the reverse
git     = "src/git/: all gix access; MUST NOT import nu_protocol/nu_plugin (grep src/git for nu_ must stay empty); no Span, Value, LabeledError, CustomValue"
nushell = "src/commands/ + src/cdt/ (custom data types): PluginCommands, CustomValues, Value/record mapping, LabeledError conversion; MUST NOT touch gix types (tests may call gix to build fixtures)"
api     = "git module exposes domain types (CommitInfo, GixError) and plain-Rust fns/iterators; gix types may live inside git-side state objects but never cross into src/cdt or src/commands; mod.rs only declares modules and re-exports"
# exception: gix::ObjectId and gix::revision::plumbing::Spec may appear in git-side
# domain types that serialize into cdt handles (e.g. Side). Justification: the workspace
# controls the compiled feature set (sha1 only, bumps deliberate), so the enums are total
# in practice; they are #[non_exhaustive] only against future gix additions, and gix
# versions are pinned; they carry plain digests and derive Copy/Eq/serde (Spec is
# documented by gix as suitable for serialization and thread movement). Repository-bound
# gix types (Repository, an attached Spec, ...) remain banned from serialized handles.
errors  = "git::GixError is Nushell-free; commands::CommandError wraps it (#[from]) plus argument errors (e.g. NegativeLimit) and owns into_labeled(span), called only at run/stream-mapping boundaries"
values  = "cdt values are stateful handles: they serialize only an id plus the self-describing fields needed to rebuild their state (git dir, sides, oids, validity stamp); the state itself (may hold gix types) lives plugin-side in a git-side registry that is only a cache"
state   = "registry entries are Arc'd immutable snapshots + a stamp; every use validates the stamp: content-addressed state never goes stale; live state (index/worktree/refs/config) is revalidated on reads and compare-and-swap on writes (stale-changeset, nothing applied)"
gc      = "Nushell GC stops idle plugins (default 10s) and live custom values do NOT keep a plugin alive: an unknown id means rebuild from the handle's fields, never a bare error; stale-handle/repository-gone only when rebuilding is impossible; drop is signalled via notify_plugin_on_drop -> Plugin::custom_value_dropped; set_gc_disabled is an optional tuning step, not a correctness mechanism"
scratch = "files owned by handles (scratch indexes) live in a per-plugin temp dir and are swept for orphans at startup; a killed plugin gets no drop notification"
streams = "git yields Iterator<Item = Result<DomainType, GixError>>; commands map items to Value and wrap in ListStream with engine.signals()"
doctest = "bin crate => no doctests; if the git side needs tested examples, extract src/git into a lib target (compile-time enforcement of this boundary)"

[antirot]
rules = "del dead-code (no commented-out blocks; no #[expect(dead_code)] on prod items); no-TODO (CI greps for TODO/FIXME/HACK/XXX in *.rs); ≥2-callers; docs-same-commit; tests-guard-intent; no-defensive-impossible"
# exception: nu-plugin-gix-testing-support is exempt from ≥2-callers — a fixture there may have a single user,
# because it encodes git's on-disk layout and keeps that knowledge out of the test modules

[workflow]
# Every arrow is a BLOCKING gate — do not proceed until the current step is complete
steps = "Explore(read code before writing any) → Plan(>3 files: write plan+get confirmation) → Test(new behavior→failing tests first; regression→reproducing test BEFORE fix) → Impl → check(fix every warning before continuing) → fmt → Docs(update //! for every pattern change)"
cmd   = "cargo fmt --check && cargo check --all-targets && cargo clippy --all-targets && cargo test && cargo deny check"

[examples]
suppress = '''
#[expect(clippy::too_many_lines, reason = "command dispatch is inherently wide")]
#![cfg_attr(test, expect(
    clippy::panic_in_result_fn,
    reason = "rstest cases return Result for ?-based setup but use assert_matches! for assertions"
))]
'''
imports = '''
use crate::error::GixError;                     // 1. crate-internal

use nu_protocol::{LabeledError, Span};          // 2. external crates

use std::collections::BTreeMap;                 // 3. std
use std::fmt;
'''
test_error = '''
#[derive(thiserror::Error, Debug)]
enum TestError {
    #[error(transparent)] Gix(#[from] GixError),
    #[error("Test failure: {0}")] Failure(String),
}
'''
test_structure = '''
#[cfg(test)]
mod tests {
    use super::*;
    mod command { use super::*; }
    mod repo {
        use super::*;
        mod open { use super::*; }
    }
}
'''
````
