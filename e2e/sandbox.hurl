# The security claim: with sandbox on (the default), sed's three escape-hatch
# commands are rejected while the script is compiled, before any input is read.
# Tested in both directions so a regression that silently disables the sandbox
# is caught.

# w — write an arbitrary file
POST {{baseurl}}/tools/sed
{"arguments": {"script": "w {{test_dir}}/pwned.txt", "input": "x\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
jsonpath "$.error.message" contains "sandbox"

# r — read an arbitrary file
POST {{baseurl}}/tools/sed
{"arguments": {"script": "r /etc/passwd", "input": "x\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 422
[Asserts]
jsonpath "$.error.message" contains "sandbox"

# e — execute a shell command
POST {{baseurl}}/tools/sed
{"arguments": {"script": "1e echo hi", "input": "x\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 422
[Asserts]
jsonpath "$.error.message" contains "sandbox"

# The file the sandboxed w command was refused must not exist.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "p", "paths": ["{{test_dir}}/pwned.txt"], "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 404
[Asserts]
jsonpath "$.error.kind" == "std:not-found"

# Opting out lets w through, still bounded by the filesystem grant.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "w {{test_dir}}/allowed.txt", "input": "written\n", "sandbox": false},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200

POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "p", "paths": ["{{test_dir}}/allowed.txt"], "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.output" == "written\n"
