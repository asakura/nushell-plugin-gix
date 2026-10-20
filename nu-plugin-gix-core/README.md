# nu-plugin-gix-core

Pure git layer for [nushell-plugin-gix](https://github.com/asakura/nushell-plugin-gix).

This crate wraps [`gix`](https://github.com/GitoxideLabs/gitoxide) and exposes
a small, Nushell-agnostic API:

- `RepositoryLocation::discover` — find a repository on disk
- `Registry` / `OpenRepository` — bounded LRU cache of opened repositories
- `log` / `LogSettings` / `LogStart` / `CommitInfo` / `Pathspec` / `pathspec` — streaming commit history from HEAD, one or more revisions or ranges (with `all`, every reference as well), with path filters (paths resolved against a directory), first-parent, skip/limit, reverse and mailmap-mapped authors and committers (`Mailmap::Apply`, or `Mailmap::Ignore` to keep them as recorded), and with `files`, the files each commit changes against its first parent (`FileChange` with a `FileChangeKind` of added, deleted or modified), limited to the paths, and with `stat`, which implies `files` and gives each file its `FileStat` (the lines inserted and deleted against the first parent, `Binary` when either side is binary, or `Unavailable` when a blob could not be read; renames are not tracked, so a moved file counts in full)
