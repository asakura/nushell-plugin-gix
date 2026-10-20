# nu_plugin_gix

A [Nushell](https://www.nushell.sh) plugin exposing [gix](https://github.com/GitoxideLabs/gitoxide) (gitoxide).

Built against Nushell 0.112.2 and gix 0.89.

## Sides

A side names where a command reads content from, or writes content to.

| Side | Meaning | Mutable | Live |
| --- | --- | --- | --- |
| `head` | The commit currently selected by `HEAD`. | No | No |
| `index` | The staged snapshot, or a scratch index. | Yes | Yes |
| `worktree` | Files on disk in the working tree. | Yes | Yes |
| `"<rev>"` | A Git revision expression: branch, tag, object ID, or revision range. | No | No |

- `HEAD` is accepted as an alias for `head`.
- A revision can also be written as a record with a single `rev` field: `{ rev: "main" }`.
- Revisions are resolved by Git against the repository. Blank or whitespace-only revisions are rejected. A full-length object ID (40 hexadecimal characters for SHA-1, 64 for SHA-256) must be valid hexadecimal; shorter spellings, including abbreviated object IDs, are passed to resolution as written.
- Sides are independent of path selection. `--paths` is a separate list that limits a command's scope; a side never implies a path filter.
- Commands that read the working tree honor `.gitignore`, `.git/info/exclude` and global excludes. Explicit `--paths` remain available for operations that need ignored files.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
