# sed

Stream editing as an ACT component — the [uutils implementation of
`sed`](https://github.com/uutils/sed) running inside a wasm sandbox.

```bash
act call ghcr.io/actpkg/sed:0.1.0 sed \
  --args '{"script":"s/foo/bar/g","input":"foo baz foo\n"}' \
  --grant '{"wasi:filesystem":{"mode":"allowlist","allow":[{"path":"/tmp/**","mode":"rw"}]}}'
```

## Tools

| Tool | Purpose |
|---|---|
| `sed` | Transform a string. Text in, text out. |
| `sed_files` | Run a script over files, returning text or editing them in place. |

Both accept the same flags (`sandbox`, `quiet`, `extended_regexp`, `separate`,
`posix`, `null_data`, `line_length`, `debug`, `character_mode`). See
[`skill/SKILL.md`](skill/SKILL.md) for the full reference and examples.

`sed_files` processes each file independently, equivalent to `sed -s`.

## Sandbox mode

A sed script is a program: `e` runs a shell command, `r` reads any file, `w`
writes any file. This component sets sed's `--sandbox` by default, which rejects
all three *while the script is compiled*, before any input is read.

```
$ act call … sed --args '{"script":"w /tmp/pwned","input":"x\n"}'
Error: std:invalid-args: sed: <script argument 1>:1:1: error: command not allowed with --sandbox
```

Pass `"sandbox": false` when a script legitimately needs `r`/`w`; the filesystem
grant still bounds where they can reach. The `e` command stays impossible either
way — there is no shell inside the sandbox to execute.

## Capabilities

Declares `wasi:filesystem`. Both tools need it, including `sed`: the upstream
engine exposes no in-memory entry point, so text transforms are performed by
writing to a scratch file and editing it in place.

- `sed` needs only a **writable scratch directory** (default `/tmp`, override
  with the `scratch-dir` call metadata) — not access to any of your files.
- `sed_files` additionally needs access to the files it reads or edits.

Under the default `ask` policy a headless run without a grant degrades to deny;
the resulting error names the exact flag to pass.

## Building

```bash
just init      # fetch WIT deps
just build     # cargo build --target wasm32-wasip2
just pack      # embed act:component + act:skill
just test      # e2e against `act run --http`
```

Earlier revisions built for `wasm32-wasip1` and adapted the result into a
preview2 component, because `uucore` reached for `std::os::wasi::ffi` — stable
on p1, but gated behind an unstable feature on p2. uucore 0.10.0 fixed that and
`uutils/sed` picked it up, so this component now targets `wasm32-wasip2`
directly like every other one, with no adapter.

## Upstream

Pinned to `uutils/sed` at
[`36a1cf4`](https://github.com/uutils/sed/commit/36a1cf49547f5134ccd296fa3c5d3d3675ccc5e4)
rather than the crates.io release, which is materially behind. Pinning by
revision keeps builds reproducible.

## License

MIT OR Apache-2.0. Upstream `uutils/sed` is MIT.
