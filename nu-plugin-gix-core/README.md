# nu-plugin-gix-core

Pure git layer for [nushell-plugin-gix](https://github.com/asakura/nushell-plugin-gix).

This crate wraps [`gix`](https://github.com/GitoxideLabs/gitoxide) and exposes
a small, Nushell-agnostic API:

- `discover` / `RepositoryLocation` — find a repository on disk
- `Registry` / `OpenRepository` — bounded LRU cache of opened repositories
- `log` / `CommitInfo` — streaming commit history
