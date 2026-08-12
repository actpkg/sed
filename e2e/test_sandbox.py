"""The security claim: with sandbox on (the default), sed's three escape-hatch
commands are rejected while the script is compiled, before any input is read.
Tested in both directions so a regression that silently disables the sandbox
is caught.
"""

import pytest

# w — write an arbitrary file; r — read an arbitrary file; e — execute a
# shell command. All three are rejected at compile time with a message that
# names the sandbox, regardless of which one is used.
SANDBOXED_SCRIPTS = [
    pytest.param("w {tmp}/pwned.txt", id="w_write_arbitrary_file"),
    pytest.param("r /etc/passwd", id="r_read_arbitrary_file"),
    pytest.param("1e echo hi", id="e_execute_shell_command"),
]


@pytest.mark.parametrize("script_template", SANDBOXED_SCRIPTS)
async def test_sandbox_rejects_escape_commands(client, tmp_path, scratch, script_template):
    script = script_template.format(tmp=tmp_path)
    result = await client.call_tool(
        "sed",
        {"script": script, "input": "x\n", "_meta": scratch},
        raise_on_error=False,
    )
    assert result.is_error
    assert "sandbox" in result.content[0].text


async def test_sandbox_rejection_kind_is_invalid_args(client, tmp_path, scratch, expect_error):
    await expect_error(
        client, "sed",
        {"script": f"w {tmp_path}/pwned.txt", "input": "x\n", "_meta": scratch},
        "std:invalid-args",
    )


async def test_sandboxed_write_did_not_happen(client, tmp_path, scratch, expect_error):
    """The file the sandboxed w command was refused must not exist."""
    await expect_error(
        client, "sed_files",
        {
            "script": "p", "paths": [str(tmp_path / "pwned.txt")], "quiet": True,
            "_meta": scratch,
        },
        "std:not-found",
    )


async def test_sandbox_false_allows_write(client, tmp_path, scratch):
    """Opting out lets w through, still bounded by the filesystem grant."""
    write_result = await client.call_tool(
        "sed",
        {
            "script": f"w {tmp_path}/allowed.txt", "input": "written\n",
            "sandbox": False, "_meta": scratch,
        },
    )
    assert not write_result.is_error

    read_result = await client.call_tool(
        "sed_files",
        {
            "script": "p", "paths": [str(tmp_path / "allowed.txt")], "quiet": True,
            "_meta": scratch,
        },
    )
    assert read_result.structured_content["output"] == "written\n"
