"""`sed`: text in, text out. One `wasi:filesystem` grant backs the scratch
file every case round-trips through (see conftest.py `scratch`), but none of
these cases touch a caller-supplied path — that is `sed_files`' job.
"""

import pytest

CASES = [
    pytest.param(
        "s/foo/bar/g", "foo baz foo\n", {}, "bar baz bar\n",
        id="basic_substitution",
    ),
    pytest.param(
        "/ERROR/p", "ok\nERROR: bad\nfine\n", {"quiet": True}, "ERROR: bad\n",
        id="quiet_plus_p_is_grep",
    ),
    pytest.param(
        "2,4p", "a\nb\nc\nd\ne\n", {"quiet": True}, "b\nc\nd\n",
        id="line_range",
    ),
    pytest.param(
        "/^#/d; /^$/d", "# note\nkeep\n\nalso\n", {}, "keep\nalso\n",
        id="delete_comments_and_blank_lines",
    ),
    pytest.param(
        r"s/^([a-z]+)=(.*)$/\2/", "key=value\n", {"extended_regexp": True}, "value\n",
        id="extended_regexp_capture_groups",
    ),
    pytest.param(
        "1!G;h;$!d", "1\n2\n3\n", {}, "3\n2\n1\n",
        id="hold_space_reverse_like_tac",
    ),
    pytest.param(
        "s/a/b/", "a", {}, "b",
        id="missing_trailing_newline_stays_missing",
    ),
    pytest.param(
        "s/a/b/", "a\n", {}, "b\n",
        id="present_trailing_newline_is_preserved",
    ),
    pytest.param(
        "s/./X/g", "café\n", {}, "XXXX\n",
        id="utf8_dot_matches_characters_not_bytes",
    ),
]


@pytest.mark.parametrize("script,input_text,flags,expected", CASES)
async def test_sed_text_transform(client, scratch, script, input_text, flags, expected):
    result = await client.call_tool("sed", {
        "script": script,
        "input": input_text,
        "_meta": scratch,
        **flags,
    })
    assert result.content[0].text == expected
