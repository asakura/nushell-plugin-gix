//! How a changed file is classified, and how much of it changed.

/// How a file differs between a commit and its first parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChangeKind {
    /// The file exists in the commit but not in its first parent.
    Addition,
    /// The file exists in the first parent but not in the commit.
    Deletion,
    /// The file exists in both, with different content or mode.
    Modification,
}

/// How much a changed file differs: the lines inserted and removed, or no counts when git cannot count them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStat {
    /// The file is text on both sides of the change, with these many lines inserted and removed.
    Text {
        /// Lines the change adds.
        insertions: u32,
        /// Lines the change removes.
        deletions: u32,
    },
    /// The file is binary on at least one side of the change, so it has no line counts.
    Binary,
    /// The blob could not be read or diffed, so the line counts are unknown.
    Unavailable,
}

/// A file a commit changes compared with its first parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    /// The repository-relative path of the file, with `/` separators.
    pub path: String,
    /// Whether the file was added, deleted or modified.
    pub kind: FileChangeKind,
    /// The line counts of the change, or `None` when they were not requested.
    pub stat: Option<FileStat>,
}
