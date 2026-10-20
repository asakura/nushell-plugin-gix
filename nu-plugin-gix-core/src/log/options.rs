//! What a commit walk selects and how it orders it.
//!
//! [`LogStart`] says where to begin and [`LogSettings`] says how to order, trim and annotate the
//! result. `files` costs a tree diff per commit and `stat` also reads both blobs of each changed file,
//! so both are off by default and `stat` implies `files`.

/// Where a commit walk starts: the union of what `revisions` include and hide, plus every reference with `all`;
/// `HEAD` when neither is given (a hiding revision alone starts nowhere, like `git log ^x`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogStart {
    /// Revisions or ranges as written: `main`, `v1..v2`, `a...b`, `^x`, `x^@`, `x^!`.
    pub revisions: Vec<String>,
    /// Also start from every reference that peels to a commit and from `HEAD`, like `git log --all`.
    /// References that cannot be read or peeled are skipped; an empty repository yields no commits.
    pub all: bool,
}

/// Whether signatures are shown through the repository's mailmap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mailmap {
    /// Map names and e-mail addresses through `.mailmap`, `mailmap.file` and `mailmap.blob`, like `git log`.
    #[default]
    Apply,
    /// Show them as recorded, like `git log --no-mailmap`.
    Ignore,
}

/// How a walk orders, trims and annotates the commits it selects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent git log switch; grouping them would obscure the options"
)]
pub struct LogSettings {
    /// Follow only the first parent of merge commits; with `paths`, a merge is compared only with its first parent.
    pub first_parent: bool,
    /// Stream the selected commits oldest first; applied after `skip` and `limit`, so the
    /// selection is buffered.
    pub reverse: bool,
    /// Commits to leave out from the start of the selection, counted after path filtering.
    pub skip: usize,
    /// At most this many commits after `skip`; `None` means no limit.
    pub limit: Option<usize>,
    /// Whether the author and committer signatures of each commit are mapped through the mailmap.
    pub mailmap: Mailmap,
    /// Whether each listed commit carries the files it changes against its first parent, in
    /// [`CommitInfo::files`](super::CommitInfo::files), limited to `paths`; costs a tree diff per commit.
    pub files: bool,
    /// Whether each changed file carries its line counts as [`FileChange::stat`](super::FileChange::stat);
    /// implies `files`, and costs reading both blobs of every changed file.
    pub stat: bool,
}
