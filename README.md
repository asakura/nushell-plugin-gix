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

`gix config get <key>` reads one value, or one section or subsection as a record, from the repository's git configuration, for example `gix open | gix config get user.name`.

- The key is `section.name` or `section.subsection.name`; section and name match case-insensitively. The subsection is everything after the first dot, and it may contain dots.
- The value comes from the system, global, local and worktree configuration and their includes, as resolved when the repository was opened; `gix open` does not isolate the repository, so includes are followed. A key set in several layers has the value of the highest-priority layer, as with `git config --get`, unless it is a multi-value key (see below).
- Without `--type`, a key of the plugin's known-key table is converted to the kind listed there: bool (`true`, `yes`, `on`, `false`, `no`, `off`), int (as git reads integers, such as `1`, `0x10` or `2k`) or text. Every other key is text, so `user.name = 1k` gives the string `"1k"`. The keys of each kind are listed by `gix config get --help`.
- Git has no marker for keys that take several values, so the plugin keeps a fixed table of them. Without `--type`, a key on that table is always a list of its values in load order across all layers, even with one value, and each value is converted the same way. For example, `gix open | gix config get remote.origin.fetch` gives `["+refs/heads/*:refs/remotes/origin/*"]`. Run `gix config get --help` for the table: the list keys, the list keys that an empty value resets, and the bool and int keys. Key names are matched without regard to case.
- Every other key is a single value: the value of its last occurrence, even when it is set several times. Keys you define yourself are such scalars too, so `[gixtest] a = 1` and `a = 2` gives `2`.
- A valueless key (a name without `=`, as in `[gixtest]` followed by `implicit`) has no value: without `--type` it reads as `null`, where git prints an empty string. With `--type bool` it is `true`, as `git config --type=bool` prints it.
- A key that no layer sets returns `null`. A key whose values were all reset away is an error, `gix::config-key-reset`, with or without `--type`, where git prints an empty string. A section or subsection holding only such keys is the empty list `[]`. For example, `credential.helper = a` followed by `credential.helper =` makes `credential.helper` a reset key, and `gix config list` leaves it out.
- Without `--type`, a key without a dot names a section: `gix open | gix config get core` returns the record of the section's variables and subsections, nested as in `gix config list`, or `null` when no layer sets the section.
- Without `--type`, a dotted key that no layer sets as a variable names a subsection: `gix open | gix config get remote.origin` returns the record of `[remote "origin"]`. A variable is read first, so a variable and a subsection of the same name give the variable.
- A variable and a subsection with one name in one section (`[a] x = 1` with `[a "x"]`) are an error, `gix::config-name-clash`, when that section is read. Git has no groups, so the plugin reports the clash where a record would hide one variable; reading the variable itself still works.
- With `--type`, only a dotted key is read, and only its last value, never a list; a key without a dot is `gix::invalid-config-key`, and no section is returned.
- `--type` (short `-t`) takes `bool`, `int`, `path` or `string` and converts the value as git does for that type:
  - `bool`: `yes`, `on`, `true`, `no`, `off`, `false` and integers (non-zero is true); a valueless key is `true`.
  - `int`: integers as git reads them, including `0x` hexadecimal, and `k`, `m` and `g` suffixes (powers of 1024). A valueless key is an error, `gix::config-value-missing`, where git fails.
  - `path`: `~` and `%(prefix)/` are expanded. Values set in sections of untrusted files (a repository you do not own) are ignored. An `:(optional)` path that does not exist is `null`. A valueless key is an error, `gix::config-value-missing`, where git fails.
  - `string`: the raw value. Git has no `string` type (`git config get --type=string` fails with `unrecognized --type argument`), so this type is an extension of this plugin. A valueless key is an error, `gix::config-value-missing`.
- A value that does not convert to the requested `--type` is an error, not `null`.
- A value that is not valid UTF-8 is an error only for a read that reaches it: `gix config get user.name` works when another subsection of the configuration is not UTF-8.
- Example: `gix open | gix config get core.bare --type bool`.
- Writes are not supported.
- Errors: `gix::invalid-config-key` for a key that is not a dotted name, when `--type` is given; `gix::config-value-not-utf8` for a value that is not valid UTF-8; `gix::config-subsection-not-utf8`, `gix::config-section-unreadable` and `gix::config-section-malformed` for a section or subsection that cannot be read as a record; `gix::config-name-clash` for a variable and a subsection of one name; `gix::unknown-config-type` for a `--type` other than `bool`, `int`, `path` or `string`; `gix::invalid-config-value` for a value that does not convert to the requested `--type`, or, read without `--type`, for a value of a known bool or int key that does not convert, such as `[gc] auto = banana`; `gix::config-value-missing` for a valueless key read with `--type int`, `path` or `string`; `gix::config-key-reset` for a key whose values were all reset away; `gix::config-path-not-utf8` for an expanded `path` that is not valid UTF-8; and `gix::install-dir-unavailable` for a `%(prefix)/` path when git's install directory cannot be found.
- An error about a value names where the value came from: its file, as in `(in /home/u/.gitconfig)`, or else its source kind, as in `(from the environment)`. The code does not change.

`gix config list` lists the whole layered configuration as one nested record, for example `gix open | gix config list | get core.editor`.

- The record nests by section: each section is a column whose record holds its variables, and a subsection is a record inside its section, so `remote.origin.url` is `remote`, then `origin`, then `url`.
- Entries come from the system, global, local and worktree configuration and their includes. A key set in several layers has the value of the highest-priority layer, as with `git config --get`, unless it is a multi-value key (see `gix config get`).
- Section and variable names are lowercase. A subsection keeps its case, except in a legacy `[section.subsection]` header, where it is lowercase too.
- Each value is converted as `gix config get` converts it without `--type`: a bool, an int or a string. A valueless key is `null`, also as an element of a list.
- A key on the multi-value table of `gix config get` is a list of its values in load order across all layers. Any other key is the value of its last occurrence.
- A list key whose values were all reset away is left out, as a key no layer sets is.
- A variable and a subsection with one name in one section are an error, `gix::config-name-clash`, and the whole listing fails, since git has no groups. Deliberate deviation: git lists both.
- Columns are alphabetical, not in load order, and no scope or file is reported.
- Errors: `gix::wrong-input`, `gix::invalid-config-value` (for a value of a known bool or int key that does not convert, such as `[gc] auto = banana`), `gix::config-value-not-utf8`, `gix::config-subsection-not-utf8`, `gix::config-section-unreadable`, `gix::config-section-malformed`, `gix::config-name-clash`, `gix::open-failed` and `gix::repository-gone`. The errors about an entry point at the piped repository handle, since `list` takes no key argument, and they name the file or source the entry came from, as `gix config get` does.

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
