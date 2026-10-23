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

## Config

`gix config [--type <t>] [<key>]` reads the repository's git configuration. Without a key it returns the whole configuration as one nested record, for example `gix open | gix config | get remote.origin.url`; with a key it returns that section, subsection or variable, for example `gix open | gix config user.name`.

- The value comes from the system, global, local and worktree configuration and their includes, as resolved when the repository was opened; `gix open` does not isolate the repository, so includes are followed. A key set in several layers has the value of the highest-priority layer, as with `git config --get`, unless it is a multi-value key (see below).
- The record nests by section: each section is a column whose record holds its variables, and a subsection is a record inside its section, so `remote.origin.url` is `remote`, then `origin`, then `url`. Columns are alphabetical.
- Section and variable names are lowercase. A subsection keeps its case, except in a legacy `[section.subsection]` header, where it is lowercase too.
- Without `--type`, a key of the plugin's known-key table is converted to the kind listed there: bool (`true`, `yes`, `on`, `false`, `no`, `off`), int (as git reads integers, such as `1`, `0x10` or `2k`) or text. Every other key is text, so `user.name = 1k` gives the string `"1k"`. The keys of each kind are listed by `gix config --help`.
- Git has no marker for keys that take several values, so the plugin keeps a fixed table of them. A key on that table is always a list of its values in load order across all layers, even with one value, and each value is converted the same way. For example, `gix open | gix config remote.origin.fetch` gives `["+refs/heads/*:refs/remotes/origin/*"]`. Run `gix config --help` for the table: the list keys, the list keys that an empty value resets, and the bool and int keys. Key names are matched without regard to case.
- Every other key is a single value: the value of its last occurrence, even when it is set several times. Keys you define yourself are such scalars too, so `[gixtest] a = 1` and `a = 2` gives `2`.
- A valueless key (a name without `=`, as in `[gixtest]` followed by `implicit`) has no value: without `--type` it reads as `null`, where git prints an empty string, also as an element of a list. With `--type bool` it is `true`, as `git config --type=bool` prints it.
- The key is a Nushell cell path or a string.
  - A cell path is already split: one part is a section (`core`), two parts are `section.name` (a variable `user.name`, else the subsection `name` of the section: `remote.origin` is the record of `[remote "origin"]`), three parts are `section.subsection.name` (`remote.origin.url`). More than three parts are an error, `gix::invalid-config-key`. Quote a part that contains a dot: `url."https://a.b/".insteadof`.
  - A string, such as a variable holding text, is split at its first and last dot: the section is before the first dot, the name after the last, and the subsection between them may contain dots (`let k = 'url.https://a.b/.insteadof'; gix open | gix config $k`). A string without a dot is a section.
  - A quoted literal such as `"user.name"` is ONE cell path part, as in Nushell's `get`, so it is read as the section `user.name` and fails with `gix::invalid-config-key`. Write the bare form `user.name`, or pass a string variable.
  - Section and variable names ignore case; a subsection is matched exactly.
  - A part marked `?` (`user.nickname?`) makes a key that is not set `null` instead of an error. `!` is accepted and changes nothing, since names are already case-insensitive.
- A key that is not set is an error, `gix::config-key-not-found`, as with Nushell's `get`.
- A key whose values were all reset away is left out of every record. Asked for directly it is an error, `gix::config-key-reset`, with or without `--type`, where git prints an empty string. A section or subsection holding only such keys counts as not set. For example, `credential.helper = a` followed by `credential.helper =` makes `credential.helper` a reset key.
- A variable and a subsection with one name in one section are not an error: the record shows the subsection, and `gix config list` shows both.
- `--type` (short `-t`) needs a key (`gix::config-type-needs-key`) that names a single value; a section or subsection is `gix::config-type-needs-value`. Only the last value of the key is read, never a list. `--type` takes `bool`, `int`, `path` or `string` and converts the value as git does for that type:
  - `bool`: `yes`, `on`, `true`, `no`, `off`, `false` and integers (non-zero is true); a valueless key is `true`.
  - `int`: integers as git reads them, including `0x` hexadecimal, and `k`, `m` and `g` suffixes (powers of 1024). A valueless key is an error, `gix::config-value-missing`, where git fails.
  - `path`: `~` and `%(prefix)/` are expanded. Values set in sections of untrusted files (a repository you do not own) are ignored. An `:(optional)` path that does not exist is `null`. A valueless key is an error, `gix::config-value-missing`, where git fails.
  - `string`: the raw value. Git has no `string` type (`git config --type=string` fails with `unrecognized --type argument`), so this type is an extension of this plugin. A valueless key is an error, `gix::config-value-missing`.
- A value that does not convert to the requested `--type` is an error, not `null`. The git types `color`, `size` and `expiry-date` are not supported yet (issues #120 for `size` and #121 for `expiry-date`).
- A value that is not valid UTF-8 is an error only for a read that reaches it: `gix config user.name` works when another subsection of the configuration is not UTF-8, while the whole record does not.
- Example: `gix open | gix config core.bare --type bool`.
- Writes are not supported.
- Errors: `gix::invalid-config-key` for a key with more than three cell path parts or a malformed key; `gix::config-key-not-found` for a key that nothing sets; `gix::config-type-needs-key` for `--type` without a key; `gix::config-type-needs-value` for `--type` with a section or subsection; `gix::config-value-not-utf8` for a value that is not valid UTF-8; `gix::config-subsection-not-utf8`, `gix::config-section-unreadable` and `gix::config-section-malformed` for a section or subsection that cannot be read as a record; `gix::unknown-config-type` for a `--type` other than `bool`, `int`, `path` or `string`; `gix::invalid-config-value` for a value that does not convert to the requested `--type`, or, read without `--type`, for a value of a known bool or int key that does not convert, such as `[gc] auto = banana`; `gix::config-value-missing` for a valueless key read with `--type int`, `path` or `string`; `gix::config-key-reset` for a key whose values were all reset away; `gix::config-path-not-utf8` for an expanded `path` that is not valid UTF-8; and `gix::install-dir-unavailable` for a `%(prefix)/` path when git's install directory cannot be found.
- An error about a value names where the value came from: its file, as in `(in /home/u/.gitconfig)`, or else its source kind, as in `(from the environment)`. The code does not change.
- Breaking changes: the former subcommand for reading one key is gone; use `gix config <key>`. A variable and a subsection of one name are no longer an error. A section holding only reset keys is not set instead of `[]`. A quoted `"user.name"` literal is one cell path part and is invalid; pass a string variable or the bare dotted form.

`gix config list` lists every setting of the layered configuration as a table, for example `gix open | gix config list | where scope == local`.

- Columns are `key` (lowercase section and variable name, subsection case kept: `remote.Origin.url`), `value`, `scope` and `file`.
- There is one row per occurrence, as `git config --list --show-origin --show-scope` shows them: system, global, local and worktree files and their includes, then the command line and environment. Nothing is merged: a variable and a subsection of one name both appear, and a list key shows every value.
- Rows are sorted by key, and the occurrences of one key are in load order, so the last row of a key is the value git uses.
- `value` is always text, with no conversion; it is `null` for a name written without `=`.
- `scope` is `system`, `global`, `local`, `worktree` or `command`. `file` is the path of the file, or `null` when the setting does not come from a file.
- Errors: `gix::wrong-input`, `gix::config-path-not-utf8`, `gix::config-value-not-utf8`, `gix::config-subsection-not-utf8`, `gix::config-section-unreadable`, `gix::config-section-malformed`, `gix::open-failed` and `gix::repository-gone`. A value or subsection that is not UTF-8 fails the whole listing, and the error names the file or source the entry came from.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
