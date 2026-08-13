"""Shared fixtures for the MCP-driven e2e suite.

The suite drives the packed component through `act run --mcp` over stdio with
a real MCP client, so what the tests observe is what an agent observes.
"""

import asyncio
import json
import os
import shlex
import subprocess
import pytest
from contextlib import AsyncExitStack
from pathlib import Path

from fastmcp import Client
from fastmcp.client.transports import StdioTransport

# Measured in docs/specs/2026-08-08-e2e-harness-findings.md, question 1.
from mcp.shared.exceptions import McpError

WASM = "target/wasm32-wasip2/release/component_sed.wasm"

# ACT's audit trail writes to stderr unconditionally — it is not governed by
# RUST_LOG — so it is redirected to a file rather than left to flood pytest.
LOG_FILE = Path(".pytest-act-stderr.log")

# Deliberately loose. `act run --mcp` instantiates the component before it
# answers `initialize`, so "connect" includes that cost -- for a heavy
# component (servo embeds a browser engine) it is seconds, and on a loaded
# runner it varies. 30s tripped servo in CI while its healthy connect was
# ~8s, so the bound sits well above the worst observed cost and still well
# below the per-test timeout, keeping this the diagnostic that fires first.
CONNECT_TIMEOUT = 120


@pytest.fixture(scope="session")
def act_command() -> list[str]:
    """The ACT invocation, honouring the same override the justfile uses.

    Parsed with shlex, not treated as a single path: the justfile's own
    default for its `act` variable is `npx @actcore/act` — two words — which
    cannot be `argv[0]` for a non-shell `subprocess.run`/`StdioTransport`
    call. A bare `os.environ.get("ACT", "act")` string breaks that default;
    splitting it is what makes both forms ("act" on PATH, and the npx
    two-word default) actually spawn.
    """
    return shlex.split(os.environ.get("ACT", "act"))


@pytest.fixture(scope="session")
def wasm_path(act_command: list[str]) -> Path:
    """The packed component.

    Existence is not enough and neither is a fresh mtime: `cargo build`
    produces a wasm with no `act:component` custom section, and an unpacked
    artifact declares no capability ceiling, so every grant is refused as
    "outside ceiling" and the failures point anywhere but here. This has
    already bitten three times in this workspace, so the fixture checks the
    section rather than the file.
    """
    path = Path(WASM)
    if not path.exists():
        pytest.fail(f"{path} is missing — run `just build && just pack` first")
    probe = subprocess.run(
        [*act_command, "inspect", "component-manifest", str(path)],
        capture_output=True, text=True,
    )
    name = json.loads(probe.stdout or "{}").get("std", {}).get("name", "unknown")
    if name in ("", "unknown"):
        pytest.fail(f"{path} is built but not packed — run `just pack`")
    return path


@pytest.fixture
async def client(act_command: list[str], wasm_path: Path, tmp_path: Path):
    """A connected MCP client, one `act` process AND one private directory per test.

    `sed` needs a `wasi:filesystem` grant even for its plain-text tool: it has
    no in-memory entry point into the underlying uutils implementation, so
    every transform round-trips through a scratch file (see src/lib.rs). It is
    therefore stateful across *files*, not just across calls within a
    process — the same reasoning `filesystem`'s reference fixture documents.
    The five old hurl files shared a single `mktemp -d` for the entire suite
    run; giving each test its own `act` process and its own directory is
    strictly tighter isolation, not a like-for-like port of that.

    `tmp_path` is pytest's own function-scoped temp-directory fixture. Taking
    it here (and letting each test take it too) means the same directory
    instance backs both the grant built below and whatever paths a test
    constructs — pytest caches a fixture's value per test, so `client`'s
    `tmp_path` and a test function's `tmp_path` parameter are guaranteed to be
    the identical directory.

    Grant shape carried verbatim from the old justfile's `--grant` (mode
    `allowlist`, `rw`, path = the private directory, no `/**` suffix needed —
    `wasi:filesystem` treats it as a subtree root).
    """
    grant = json.dumps({
        "wasi:filesystem": {
            "mode": "allowlist",
            "allow": [{"path": str(tmp_path), "mode": "rw"}],
        }
    })
    transport = StdioTransport(
        command=act_command[0],
        args=[*act_command[1:], "run", str(wasm_path), "--mcp", "--grant", grant],
        keep_alive=False,  # stateful component: fresh process per test is not optional here
        log_file=LOG_FILE,
    )
    async with AsyncExitStack() as stack:
        # Bound the connect, not the test body. A stalled handshake otherwise
        # consumes the whole pytest timeout with no diagnostic at all — which
        # is precisely how the webdriver-bidi CI hang presented for hours.
        try:
            async with asyncio.timeout(CONNECT_TIMEOUT):
                connected = await stack.enter_async_context(Client(transport))
        except TimeoutError:
            pytest.fail(
                f"MCP client did not connect within {CONNECT_TIMEOUT}s; "
                f"act's stderr, if it wrote any, is dumped at session end"
            )
        yield connected


@pytest.fixture
def scratch(tmp_path: Path) -> dict:
    """The `_meta` argument-channel payload every `sed`/`sed_files` call needs.

    `sed`'s default scratch directory is `/tmp` (see src/lib.rs
    `default_scratch_dir`), but the grant built by the `client` fixture above
    only covers `tmp_path`, not all of `/tmp` — so every call must point the
    component's scratch directory at the one directory it is actually granted.
    This mirrors the old hurl suite's `"metadata": {"scratch-dir": "{{test_dir}}"}}`
    on (almost) every request, carried into the MCP argument metadata channel
    (ACT-MCP §3.2) under its un-namespaced key: `scratch-dir` is a
    component-specific key, not one of the `std:*` well-known ones.
    """
    return {"scratch-dir": str(tmp_path)}


@pytest.fixture
def expect_error():
    """Assert a call fails with a specific ACT error kind.

    Exposed as a fixture rather than a plain function so tests never have to
    import from `conftest` — that import only resolves when the test
    directory happens to be on `sys.path`, which is not something to rely on.

    Measured, not assumed. `call-tool` in `act:tools` returns a bare
    `tool-result` with NO `result<>` wrapper — only `list-tools` has one — so
    a guest reporting a failed tool call can only do it through
    `tool-event::error`, which arrives as a result with `is_error` set and the
    kind in `_meta`. **That is the path a tool test will take.**

    The JSON-RPC error path exists for failures that are not the guest's tool
    body: `list-tools`, the session operations, a wasmtime trap, an
    unreachable actor. It raises `mcp.shared.exceptions.McpError` with the
    payload at `exc.error.data`. Session-lifecycle tests reach it; a
    malformed-script test does not. Both are handled here so callers need
    not care.
    """

    async def _expect(client, tool: str, arguments: dict, kind: str):
        try:
            result = await client.call_tool(tool, arguments, raise_on_error=False)
        except McpError as exc:
            data = getattr(getattr(exc, "error", None), "data", None) or {}
            assert data.get("dev.actcore/error-kind") == kind, (
                f"expected {kind} on the JSON-RPC error path, got {data!r}"
            )
            return

        assert result.is_error, f"expected {tool} to fail, got {result!r}"
        meta = result.meta or {}
        assert meta.get("dev.actcore/error-kind") == kind, (
            f"expected {kind} on the isError path, got {meta!r}"
        )

    return _expect


def pytest_sessionfinish(session, exitstatus):
    """Print act's stderr when the run did not pass.

    `log_file` keeps the audit trail out of the test output, which is right
    for a green run and wrong for every other kind: on an ephemeral CI runner
    nothing ever reads that file. Diagnosing a CI-only hang in this fleet
    cost several rounds of probing that one line of this stream would have
    answered. A hook rather than a fixture finaliser on purpose — fixture
    teardown does not run when the session dies mid-test.
    """
    if exitstatus == 0 or not LOG_FILE.exists():
        return
    text = LOG_FILE.read_text(errors="replace").strip()
    if text:
        print(f"\n--- act stderr ({LOG_FILE}) ---\n{text}")
