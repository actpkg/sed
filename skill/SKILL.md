---
name: sed
description: Stream editing with sed — substitute, filter and transform text, or edit files in place
metadata:
  act: {}
---

# sed Component

The uutils implementation of `sed`, running inside a wasm sandbox.

## Tools

### `sed` — transform a string

```json
{ "script": "s/foo/bar/g", "input": "foo baz foo\n" }
```

Returns the transformed text. This is the tool to reach for by default: pass the
text directly, get text back.

### `sed_files` — operate on files

```json
{ "script": "/^#/d", "paths": ["/data/config.ini"], "in_place": true }
```

With `in_place: false` (the default) it returns the transformed text and leaves
the files alone. With `in_place: true` it rewrites them; set `in_place_suffix`
(e.g. `".bak"`) to keep the originals.

Each file is processed **independently**, equivalent to `sed -s`. Line numbers
and `$` apply per file, not across the whole set. If you need continuous-stream
semantics across several files, concatenate them yourself and use `sed`.

## Options

Both tools accept the same flags, each mirroring a sed command-line option:

| Option | sed flag | Default | Meaning |
|---|---|---|---|
| `sandbox` | `--sandbox` | `true` | Reject `e`, `r`, `w` at compile time |
| `quiet` | `-n` | `false` | Suppress automatic printing |
| `extended_regexp` | `-E` | `false` | Extended regular expressions |
| `separate` | `-s` | `false` | Treat files separately |
| `posix` | `--posix` | `false` | Disable GNU extensions |
| `null_data` | `-z` | `false` | NUL-separated lines |
| `line_length` | `-l` | `70` | Wrap width for the `l` command |
| `debug` | `--debug` | `false` | Annotate program execution |
| `character_mode` | locale | `utf8` | `utf8` or `byte` |

## Sandbox mode

`sandbox` defaults to **true**, which makes sed reject three commands while the
script is being compiled — before any input is read:

- `e` — execute a shell command
- `r` — read an arbitrary file into the output
- `w` — write the pattern space to an arbitrary file

This matters because a sed script is a program. If you are applying a script
that came from an untrusted source, leave `sandbox` on: it is the difference
between a text transform and arbitrary file access.

Set `sandbox: false` only when the script legitimately needs `r`/`w`. The
filesystem grant still bounds where those commands can reach, and `e` remains
impossible regardless — there is no shell inside the sandbox to execute.

## Capabilities

This component declares `wasi:filesystem`.

Both tools need it, including `sed` — the engine has no in-memory entry point,
so text transforms go through a scratch file. The distinction is *what* each
needs access to:

- `sed` needs only a writable scratch directory (default `/tmp`):

  ```
  --grant '{"wasi:filesystem":{"mode":"allowlist","allow":[{"path":"/tmp/**","mode":"rw"}]}}'
  ```

- `sed_files` additionally needs access to the files it is asked to read or edit.

Point the scratch directory elsewhere with the `scratch-dir` call metadata.

Without a grant, the default `ask` policy degrades to deny on a headless run and
calls will fail with a message naming the flag to pass.

## Examples

Delete comment lines and blank lines:

```json
{ "script": "/^#/d; /^$/d", "input": "# note\nkeep\n\nalso\n" }
```

Print only matching lines (`-n` plus `p`, i.e. grep):

```json
{ "script": "/ERROR/p", "input": "ok\nERROR: bad\n", "quiet": true }
```

Extract a capture group with extended regexps:

```json
{ "script": "s/^([a-z]+)=(.*)$/\\2/", "input": "key=value\n", "extended_regexp": true }
```

Print a line range:

```json
{ "script": "2,4p", "input": "a\nb\nc\nd\ne\n", "quiet": true }
```
