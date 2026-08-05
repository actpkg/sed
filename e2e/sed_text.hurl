# Basic substitution
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/foo/bar/g", "input": "foo baz foo\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "bar baz bar\n"

# -n plus p, i.e. grep
POST {{baseurl}}/tools/sed
{"arguments": {"script": "/ERROR/p", "input": "ok\nERROR: bad\nfine\n", "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "ERROR: bad\n"

# Line range
POST {{baseurl}}/tools/sed
{"arguments": {"script": "2,4p", "input": "a\nb\nc\nd\ne\n", "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "b\nc\nd\n"

# Deletion of comments and blank lines
POST {{baseurl}}/tools/sed
{"arguments": {"script": "/^#/d; /^$/d", "input": "# note\nkeep\n\nalso\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "keep\nalso\n"

# Extended regexps and capture groups
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/^([a-z]+)=(.*)$/\\2/", "input": "key=value\n", "extended_regexp": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "value\n"

# Hold space: reverse the lines, like tac
POST {{baseurl}}/tools/sed
{"arguments": {"script": "1!G;h;$!d", "input": "1\n2\n3\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "3\n2\n1\n"

# A missing trailing newline stays missing, matching GNU and uutils sed. The
# scratch-file round-trip must not introduce one.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/a/b/", "input": "a"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "b"

# ...and a present one is preserved.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/a/b/", "input": "a\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "b\n"

# UTF-8 is the default: `.` matches characters, not bytes.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/./X/g", "input": "café\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data" == "XXXX\n"
