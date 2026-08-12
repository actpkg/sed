"""`sed_files`: runs a script over files on disk, either returning the
transformed text or editing in place. The two flows (read-only concat,
in-place with backup) are sequential — each step depends on file state the
previous step created — so they stay as two multi-step tests rather than
being flattened into a parametrize table.
"""


async def test_sed_files_readonly_concat_leaves_originals_untouched(client, tmp_path, scratch):
    a = tmp_path / "a.txt"
    b = tmp_path / "b.txt"

    # Seed two input files via sed itself, writing with the w command.
    await client.call_tool("sed", {
        "script": f"w {a}", "input": "alpha\nbeta\ngamma\n",
        "sandbox": False, "quiet": True, "_meta": scratch,
    })
    await client.call_tool("sed", {
        "script": f"w {b}", "input": "delta\n",
        "sandbox": False, "quiet": True, "_meta": scratch,
    })

    # Read-only over several files: output is concatenated, originals untouched.
    result = await client.call_tool("sed_files", {
        "script": "s/a/A/g", "paths": [str(a), str(b)], "_meta": scratch,
    })
    assert result.structured_content["output"] == "AlphA\nbetA\ngAmmA\ndeltA\n"
    assert "edited" not in result.structured_content

    # The originals must be unchanged.
    result = await client.call_tool("sed_files", {
        "script": "p", "paths": [str(a)], "quiet": True, "_meta": scratch,
    })
    assert result.structured_content["output"] == "alpha\nbeta\ngamma\n"


async def test_sed_files_in_place_with_backup(client, tmp_path, scratch):
    a = tmp_path / "a.txt"
    await client.call_tool("sed", {
        "script": f"w {a}", "input": "alpha\nbeta\ngamma\n",
        "sandbox": False, "quiet": True, "_meta": scratch,
    })

    # In-place editing with a backup suffix.
    result = await client.call_tool("sed_files", {
        "script": "s/beta/BETA/", "paths": [str(a)],
        "in_place": True, "in_place_suffix": ".bak", "_meta": scratch,
    })
    assert result.structured_content["edited"] == [str(a)]
    assert "output" not in result.structured_content

    # The file is rewritten...
    result = await client.call_tool("sed_files", {
        "script": "p", "paths": [str(a)], "quiet": True, "_meta": scratch,
    })
    assert result.structured_content["output"] == "alpha\nBETA\ngamma\n"

    # ...and the backup holds the original.
    result = await client.call_tool("sed_files", {
        "script": "p", "paths": [str(a) + ".bak"], "quiet": True, "_meta": scratch,
    })
    assert result.structured_content["output"] == "alpha\nbeta\ngamma\n"


async def test_sed_files_missing_input_is_not_found(client, tmp_path, scratch, expect_error):
    """A missing input is reported as not-found, not an internal error."""
    await expect_error(
        client, "sed_files",
        {"script": "s/x/y/", "paths": [str(tmp_path / "nope.txt")], "_meta": scratch},
        "std:not-found",
    )


async def test_sed_files_empty_paths_is_invalid_args(client, scratch, expect_error):
    """An empty path list is rejected rather than panicking inside the engine."""
    await expect_error(
        client, "sed_files",
        {"script": "s/x/y/", "paths": [], "_meta": scratch},
        "std:invalid-args",
    )


async def test_sed_malformed_script_is_invalid_args(client, scratch, expect_error):
    """A malformed script is a caller error carrying the engine's own diagnostic."""
    await expect_error(
        client, "sed",
        {"script": "s/unterminated", "input": "x\n", "_meta": scratch},
        "std:invalid-args",
    )
