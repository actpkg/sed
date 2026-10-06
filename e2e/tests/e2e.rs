//! Drive the packed component through `act run --mcp` with a real MCP client.
//!
//! This translates the python fastmcp/pytest suite alongside which it lives —
//! `conftest.py`, `test_info.py`, `test_list_tools.py`, `test_sandbox.py`,
//! `test_sed_files.py`, `test_sed_text.py` — one test for one test: same
//! assertions, same error kinds (`dev.actcore/error-kind`), same `_meta`
//! keys, same grant shape. The tests observe exactly what an agent observes.
//!
//! Env: WASM — path to the packed component (default: the component's
//!      release build output);
//!      ACT  — the act invocation (default `act`; `npx @actcore/act`, the
//!             component justfile's default, also works — whitespace-split,
//!             like the shlex.split the python conftest did).

use std::path::PathBuf;
use std::time::Duration;

use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::TokioChildProcess,
};
use serde_json::{Value, json};

/// `().serve(transport)` hands back the client-role service running over the
/// child process: role first, the unit client handler second.
type Client = rmcp::service::RunningService<rmcp::service::RoleClient, ()>;

/// The python conftest's CONNECT_TIMEOUT: deliberately loose, because
/// `act run --mcp` instantiates the component before it answers
/// `initialize`. Bounding the connect — not the test body — means a stalled
/// handshake fails with a diagnostic instead of hanging the whole run.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);

fn wasm_path() -> PathBuf {
    PathBuf::from(std::env::var("WASM").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/wasm32-wasip2/release/component_sed.wasm"
        )
        .into()
    }))
}

/// The ACT invocation, honouring the same override the component justfile
/// uses. Its default there is `npx @actcore/act` — two words — which cannot
/// be `argv[0]` for a non-shell spawn, so the value is whitespace-split into
/// program + leading args. Quoted paths with spaces are not a form this
/// fleet passes through `ACT`; a full shlex is deliberately not pulled in.
fn act_argv() -> Vec<String> {
    std::env::var("ACT")
        .unwrap_or_else(|_| "act".into())
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// The per-test scratch directory (python conftest: `client` + `scratch`)
// ---------------------------------------------------------------------------

/// Removes the scratch directory when the test ends. Best-effort: a leftover
/// directory under the system temp dir is cosmetic, and a cleanup error must
/// never mask the test's own result.
struct ScratchGuard(PathBuf);

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One private scratch directory per test — the analogue of pytest's
/// function-scoped `tmp_path`, which backed both the filesystem grant and the
/// `scratch-dir` metadata within one test.
///
/// `sed` needs a `wasi:filesystem` grant even for its plain-text tool: it has
/// no in-memory entry point into the underlying uutils implementation, so
/// every transform round-trips through a scratch file (src/lib.rs
/// `run_in_scratch`). It is therefore stateful across *files*, not just
/// across calls within a process — which is why each test gets its own `act`
/// process AND its own directory (the python `client` fixture's
/// `keep_alive=False`).
struct Scratch {
    dir: PathBuf,
    _guard: ScratchGuard,
}

fn scratch(tag: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("sed-e2e-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("create the test's scratch directory");
    Scratch {
        _guard: ScratchGuard(dir.clone()),
        dir,
    }
}

impl Scratch {
    /// The `_meta` argument-channel payload every `sed`/`sed_files` call
    /// needs. The component's default scratch directory is `/tmp`
    /// (src/lib.rs `default_scratch_dir`), but the grant built for a test
    /// covers only this directory — so every call points the component's
    /// scratch directory at the one it is actually granted. This mirrors the
    /// old hurl suite's `"metadata": {"scratch-dir": "{{test_dir}}"}`
    /// carried into the MCP argument metadata channel (ACT-MCP §3.2) under
    /// its un-namespaced key: `scratch-dir` is component-specific, not one of
    /// the `std:*` well-known ones.
    fn meta(&self) -> Value {
        json!({ "scratch-dir": self.dir.display().to_string() })
    }

    /// A path inside the scratch directory, as a string — the shape the
    /// tools' `paths` arguments take.
    fn path(&self, name: &str) -> String {
        self.dir.join(name).display().to_string()
    }
}

/// Grant shape carried verbatim from the python conftest (mode `allowlist`,
/// `rw`, path = the private directory, no `/**` suffix needed —
/// `wasi:filesystem` treats it as a subtree root).
///
/// Grants are NOT optional: the default policy mode is `ask` and a headless
/// run degrades it to deny.
fn grant(s: &Scratch) -> String {
    json!({
        "wasi:filesystem": {
            "mode": "allowlist",
            "allow": [{ "path": s.dir.display().to_string(), "mode": "rw" }]
        }
    })
    .to_string()
}

fn act_command(s: &Scratch) -> tokio::process::Command {
    let argv = act_argv();
    let mut cmd = tokio::process::Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    cmd.arg("run").arg(wasm_path()).arg("--mcp");
    cmd.args(["--grant", &grant(s)]);
    cmd
}

// ---------------------------------------------------------------------------
// Client plumbing
// ---------------------------------------------------------------------------

async fn connect(s: &Scratch) -> Client {
    let transport =
        TokioChildProcess::new(act_command(s)).expect("spawn act run --mcp");
    match tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport)).await {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => panic!("rmcp handshake with act run --mcp failed: {e}"),
        Err(_) => panic!(
            "MCP client did not connect within {CONNECT_TIMEOUT:?}; the \
             handshake includes component instantiation, so first suspect the \
             component, not the transport"
        ),
    }
}

/// A reply for a call the test does not expect to fail. rmcp returns `Ok`
/// for isError results too, so the python suite's default
/// `raise_on_error=True` is kept by asserting here.
async fn call_ok(client: &Client, tool: &str, args: Value) -> rmcp::model::CallToolResult {
    let result = client
        .call_tool(
            CallToolRequestParams::new(tool.to_string())
                .with_arguments(args.as_object().expect("args must be an object").clone()),
        )
        .await
        .expect("call_tool");
    assert_ne!(result.is_error, Some(true), "call failed: {result:?}");
    result
}

/// The python conftest's `expect_error` fixture: assert a call fails with a
/// specific ACT error kind, on whichever path it arrives. `call-tool` in
/// `act:tools` returns a bare `tool-result` with NO `result<>` wrapper — only
/// `list-tools` has one — so a guest reporting a failed tool call can only do
/// it through `tool-event::error`, which arrives as a result with `is_error`
/// set and the kind in `_meta`. That is the path a tool test will take. The
/// JSON-RPC error path exists for failures that are not the guest's tool
/// body: `list-tools`, the session operations, a wasmtime trap, an
/// unreachable actor. Both are handled here so callers need not care.
async fn expect_error_kind(client: &Client, tool: &str, args: Value, kind: &str) {
    let params = CallToolRequestParams::new(tool.to_string())
        .with_arguments(args.as_object().expect("args must be an object").clone());
    match client.call_tool(params).await {
        Err(rmcp::ServiceError::McpError(e)) => {
            let got = e
                .data
                .as_ref()
                .and_then(|d| d.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str());
            assert_eq!(
                got,
                Some(kind),
                "expected {kind} on the JSON-RPC error path, got {e:?}"
            );
        }
        Ok(result) => {
            assert_eq!(
                result.is_error,
                Some(true),
                "expected {tool} to fail, got {result:?}"
            );
            let got = result
                .meta
                .as_ref()
                .and_then(|m| m.0.get("dev.actcore/error-kind"))
                .and_then(|v| v.as_str());
            assert_eq!(
                got,
                Some(kind),
                "expected {kind} on the isError path, got meta {:?} content {:?}",
                result.meta,
                result.content
            );
        }
        Err(other) => panic!("unexpected transport failure: {other:?}"),
    }
}

fn first_text_block(result: &rmcp::model::CallToolResult) -> &rmcp::model::TextContent {
    match result.content.first() {
        Some(rmcp::model::ContentBlock::Text(t)) => t,
        other => panic!("expected the first content block to be Text, got: {other:?}"),
    }
}

fn structured(result: &rmcp::model::CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("the reply must carry structured content")
}

// ---------------------------------------------------------------------------
// Argument builders (python: the `_meta` scratch fixture + per-test dicts)
// ---------------------------------------------------------------------------

/// Arguments for the `sed` tool: text in, text out. `flags` merges the
/// optional fields (`quiet`, `sandbox`, `extended_regexp`, …) on top.
fn sed_args(s: &Scratch, script: &str, input: &str, flags: Value) -> Value {
    let mut args = json!({ "script": script, "input": input, "_meta": s.meta() });
    if let (Some(dst), Some(src)) = (args.as_object_mut(), flags.as_object()) {
        dst.extend(src.clone());
    }
    args
}

/// Arguments for the `sed_files` tool: a script over files on disk.
fn sed_files_args(s: &Scratch, script: &str, paths: Vec<String>, flags: Value) -> Value {
    let mut args = json!({ "script": script, "paths": paths, "_meta": s.meta() });
    if let (Some(dst), Some(src)) = (args.as_object_mut(), flags.as_object()) {
        dst.extend(src.clone());
    }
    args
}

// ---------------------------------------------------------------------------
// test_info.py
// ---------------------------------------------------------------------------

/// The manifest probe from the python `wasm_path` fixture as well as
/// test_info.py: the packed artifact must declare its name and the
/// capability it runs under. Also the fast-fail the python fixture provided —
/// an unpacked wasm (raw `cargo build` output, no `act:component` section)
/// declares no ceiling, every grant is refused as "outside ceiling", and the
/// failures point anywhere but at the missing metadata. The justfile's
/// `test: build` ordering exists so this test finds a packed artifact.
#[test]
fn manifest_reports_name_and_capabilities() {
    let output = {
        let argv = act_argv();
        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.args(["inspect", "component-manifest"])
            .arg(wasm_path())
            .output()
            .expect("run act inspect component-manifest")
    };
    assert!(
        output.status.success(),
        "inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let manifest: Value = serde_json::from_slice(&output.stdout).expect("manifest is JSON");
    assert_eq!(
        manifest["std"]["name"], "sed",
        "packed manifest must carry the component name"
    );
    let capabilities = manifest["std"]["capabilities"]
        .as_object()
        .expect("manifest capabilities is an object keyed by capability id");
    assert!(
        capabilities.contains_key("wasi:filesystem"),
        "wasi:filesystem must be among the declared capabilities, got: {capabilities:?}"
    );
}

// ---------------------------------------------------------------------------
// test_list_tools.py
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lists_both_tools() {
    let s = scratch("list_tools");
    let client = connect(&s).await;
    let tools = client.list_all_tools().await.expect("list_all_tools");
    assert_eq!(tools.len(), 2, "exactly two tools expected: {tools:?}");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert!(
        names.contains(&"sed"),
        "sed must be among the tools, got: {names:?}"
    );
    assert!(
        names.contains(&"sed_files"),
        "sed_files must be among the tools, got: {names:?}"
    );
    client.cancel().await.ok();
}

// ---------------------------------------------------------------------------
// test_sandbox.py
//
// The security claim: with sandbox on (the default), sed's three
// escape-hatch commands are rejected while the script is compiled, before
// any input is read. Tested in both directions so a regression that silently
// disables the sandbox is caught.
// ---------------------------------------------------------------------------

/// w — write an arbitrary file; r — read an arbitrary file; e — execute a
/// shell command. All three are rejected at compile time with a message that
/// names the sandbox, regardless of which one is used.
async fn assert_sandbox_rejects(s: &Scratch, script: &str) {
    let client = connect(s).await;
    let result = client
        .call_tool(
            CallToolRequestParams::new("sed")
                .with_arguments(sed_args(s, script, "x\n", json!({})).as_object().unwrap().clone()),
        )
        .await
        .expect("call_tool");
    assert_eq!(result.is_error, Some(true), "must fail: {result:?}");
    assert!(
        first_text_block(&result).text.contains("sandbox"),
        "the rejection must name the sandbox, got: {:?}",
        result.content
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn sandbox_rejects_w_write_arbitrary_file() {
    let s = scratch("sandbox_w");
    assert_sandbox_rejects(&s, &format!("w {}/pwned.txt", s.dir.display())).await;
}

#[tokio::test]
async fn sandbox_rejects_r_read_arbitrary_file() {
    let s = scratch("sandbox_r");
    assert_sandbox_rejects(&s, "r /etc/passwd").await;
}

#[tokio::test]
async fn sandbox_rejects_e_execute_shell_command() {
    let s = scratch("sandbox_e");
    assert_sandbox_rejects(&s, "1e echo hi").await;
}

#[tokio::test]
async fn sandbox_rejection_kind_is_invalid_args() {
    let s = scratch("sandbox_kind");
    let client = connect(&s).await;
    expect_error_kind(
        &client,
        "sed",
        sed_args(
            &s,
            &format!("w {}/pwned.txt", s.dir.display()),
            "x\n",
            json!({}),
        ),
        "std:invalid-args",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn sandboxed_write_did_not_happen() {
    // The python test probes the sandboxed w command's would-be target with
    // a read-only sed_files call and expects std:not-found — that file must
    // not exist. The directory is private to this test, so the probe is the
    // whole proof.
    let s = scratch("sandbox_write_absent");
    let client = connect(&s).await;
    expect_error_kind(
        &client,
        "sed_files",
        sed_files_args(&s, "p", vec![s.path("pwned.txt")], json!({ "quiet": true })),
        "std:not-found",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn sandbox_false_allows_write() {
    let s = scratch("sandbox_off");
    let client = connect(&s).await;

    // Opting out lets w through, still bounded by the filesystem grant.
    call_ok(
        &client,
        "sed",
        sed_args(
            &s,
            &format!("w {}/allowed.txt", s.dir.display()),
            "written\n",
            json!({ "sandbox": false }),
        ),
    )
    .await;

    let read_result = call_ok(
        &client,
        "sed_files",
        sed_files_args(&s, "p", vec![s.path("allowed.txt")], json!({ "quiet": true })),
    )
    .await;
    assert_eq!(
        structured(&read_result)["output"],
        json!("written\n"),
        "the written file must read back verbatim"
    );
    client.cancel().await.ok();
}

// ---------------------------------------------------------------------------
// test_sed_files.py
//
// The two multi-step flows (read-only concat, in-place with backup) are
// sequential — each step depends on file state the previous step created —
// so they stay as multi-step tests rather than being flattened into a
// parametrize table.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sed_files_readonly_concat_leaves_originals_untouched() {
    let s = scratch("files_readonly");
    let client = connect(&s).await;
    let a = s.path("a.txt");
    let b = s.path("b.txt");

    // Seed two input files via sed itself, writing with the w command.
    call_ok(
        &client,
        "sed",
        sed_args(
            &s,
            &format!("w {a}"),
            "alpha\nbeta\ngamma\n",
            json!({ "sandbox": false, "quiet": true }),
        ),
    )
    .await;
    call_ok(
        &client,
        "sed",
        sed_args(
            &s,
            &format!("w {b}"),
            "delta\n",
            json!({ "sandbox": false, "quiet": true }),
        ),
    )
    .await;

    // Read-only over several files: output is concatenated, originals untouched.
    let result = call_ok(
        &client,
        "sed_files",
        sed_files_args(&s, "s/a/A/g", vec![a.clone(), b.clone()], json!({})),
    )
    .await;
    assert_eq!(
        structured(&result)["output"],
        json!("AlphA\nbetA\ngAmmA\ndeltA\n")
    );
    assert!(
        structured(&result).get("edited").is_none(),
        "a read-only run must not report edits: {:?}",
        structured(&result)
    );

    // The originals must be unchanged.
    let result = call_ok(
        &client,
        "sed_files",
        sed_files_args(&s, "p", vec![a], json!({ "quiet": true })),
    )
    .await;
    assert_eq!(
        structured(&result)["output"],
        json!("alpha\nbeta\ngamma\n")
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn sed_files_in_place_with_backup() {
    let s = scratch("files_inplace");
    let client = connect(&s).await;
    let a = s.path("a.txt");

    call_ok(
        &client,
        "sed",
        sed_args(
            &s,
            &format!("w {a}"),
            "alpha\nbeta\ngamma\n",
            json!({ "sandbox": false, "quiet": true }),
        ),
    )
    .await;

    // In-place editing with a backup suffix.
    let result = call_ok(
        &client,
        "sed_files",
        sed_files_args(
            &s,
            "s/beta/BETA/",
            vec![a.clone()],
            json!({ "in_place": true, "in_place_suffix": ".bak" }),
        ),
    )
    .await;
    assert_eq!(
        structured(&result)["edited"],
        json!([a]),
        "the edited path must be reported back as given"
    );
    assert!(
        structured(&result).get("output").is_none(),
        "an in-place run must not return text: {:?}",
        structured(&result)
    );

    // The file is rewritten...
    let result = call_ok(
        &client,
        "sed_files",
        sed_files_args(&s, "p", vec![a.clone()], json!({ "quiet": true })),
    )
    .await;
    assert_eq!(
        structured(&result)["output"],
        json!("alpha\nBETA\ngamma\n")
    );

    // ...and the backup holds the original.
    let result = call_ok(
        &client,
        "sed_files",
        sed_files_args(&s, "p", vec![format!("{a}.bak")], json!({ "quiet": true })),
    )
    .await;
    assert_eq!(
        structured(&result)["output"],
        json!("alpha\nbeta\ngamma\n")
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn sed_files_missing_input_is_not_found() {
    // A missing input is reported as not-found, not an internal error.
    let s = scratch("files_missing");
    let client = connect(&s).await;
    expect_error_kind(
        &client,
        "sed_files",
        sed_files_args(&s, "s/x/y/", vec![s.path("nope.txt")], json!({})),
        "std:not-found",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn sed_files_empty_paths_is_invalid_args() {
    // An empty path list is rejected rather than panicking inside the engine.
    let s = scratch("files_empty");
    let client = connect(&s).await;
    expect_error_kind(
        &client,
        "sed_files",
        sed_files_args(&s, "s/x/y/", vec![], json!({})),
        "std:invalid-args",
    )
    .await;
    client.cancel().await.ok();
}

#[tokio::test]
async fn sed_malformed_script_is_invalid_args() {
    // A malformed script is a caller error carrying the engine's own diagnostic.
    let s = scratch("malformed");
    let client = connect(&s).await;
    expect_error_kind(
        &client,
        "sed",
        sed_args(&s, "s/unterminated", "x\n", json!({})),
        "std:invalid-args",
    )
    .await;
    client.cancel().await.ok();
}

// ---------------------------------------------------------------------------
// test_sed_text.py
//
// `sed`: text in, text out. One `wasi:filesystem` grant backs the scratch
// file every case round-trips through, but none of these cases touch a
// caller-supplied path — that is `sed_files`' job.
// ---------------------------------------------------------------------------

/// The parametrize table's body. The first text block is the whole result
/// contract: exact equality, trailing newline included.
async fn assert_text_case(tag: &str, script: &str, input: &str, flags: Value, expected: &str) {
    let s = scratch(tag);
    let client = connect(&s).await;
    let result = call_ok(&client, "sed", sed_args(&s, script, input, flags)).await;
    assert_eq!(
        first_text_block(&result).text,
        expected,
        "transform output mismatch"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn sed_text_basic_substitution() {
    assert_text_case(
        "text_basic",
        "s/foo/bar/g",
        "foo baz foo\n",
        json!({}),
        "bar baz bar\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_quiet_plus_p_is_grep() {
    assert_text_case(
        "text_grep",
        "/ERROR/p",
        "ok\nERROR: bad\nfine\n",
        json!({ "quiet": true }),
        "ERROR: bad\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_line_range() {
    assert_text_case(
        "text_range",
        "2,4p",
        "a\nb\nc\nd\ne\n",
        json!({ "quiet": true }),
        "b\nc\nd\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_delete_comments_and_blank_lines() {
    assert_text_case(
        "text_delete",
        "/^#/d; /^$/d",
        "# note\nkeep\n\nalso\n",
        json!({}),
        "keep\nalso\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_extended_regexp_capture_groups() {
    assert_text_case(
        "text_extended",
        r"s/^([a-z]+)=(.*)$/\2/",
        "key=value\n",
        json!({ "extended_regexp": true }),
        "value\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_hold_space_reverse_like_tac() {
    assert_text_case(
        "text_reverse",
        "1!G;h;$!d",
        "1\n2\n3\n",
        json!({}),
        "3\n2\n1\n",
    )
    .await;
}

#[tokio::test]
async fn sed_text_missing_trailing_newline_stays_missing() {
    assert_text_case("text_no_nl", "s/a/b/", "a", json!({}), "b").await;
}

#[tokio::test]
async fn sed_text_present_trailing_newline_is_preserved() {
    assert_text_case("text_nl", "s/a/b/", "a\n", json!({}), "b\n").await;
}

#[tokio::test]
async fn sed_text_utf8_dot_matches_characters_not_bytes() {
    assert_text_case("text_utf8", "s/./X/g", "café\n", json!({}), "XXXX\n").await;
}
