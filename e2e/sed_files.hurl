# Seed two input files via sed itself, writing with the w command.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "w {{test_dir}}/a.txt", "input": "alpha\nbeta\ngamma\n", "sandbox": false, "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200

POST {{baseurl}}/tools/sed
{"arguments": {"script": "w {{test_dir}}/b.txt", "input": "delta\n", "sandbox": false, "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200

# Read-only over several files: output is concatenated, originals untouched.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "s/a/A/g", "paths": ["{{test_dir}}/a.txt", "{{test_dir}}/b.txt"]},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.output" == "AlphA\nbetA\ngAmmA\ndeltA\n"
jsonpath "$.content[0].data.edited" not exists

# The originals must be unchanged.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "p", "paths": ["{{test_dir}}/a.txt"], "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.output" == "alpha\nbeta\ngamma\n"

# In-place editing with a backup suffix.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "s/beta/BETA/", "paths": ["{{test_dir}}/a.txt"], "in_place": true, "in_place_suffix": ".bak"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.edited" count == 1
jsonpath "$.content[0].data.output" not exists

# The file is rewritten...
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "p", "paths": ["{{test_dir}}/a.txt"], "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.output" == "alpha\nBETA\ngamma\n"

# ...and the backup holds the original.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "p", "paths": ["{{test_dir}}/a.txt.bak"], "quiet": true},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 200
[Asserts]
jsonpath "$.content[0].data.output" == "alpha\nbeta\ngamma\n"

# A missing input is reported as not-found, not an internal error.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "s/x/y/", "paths": ["{{test_dir}}/nope.txt"]},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 404
[Asserts]
jsonpath "$.error.kind" == "std:not-found"

# An empty path list is rejected rather than panicking inside the engine.
POST {{baseurl}}/tools/sed_files
{"arguments": {"script": "s/x/y/", "paths": []},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"

# A malformed script is a caller error carrying the engine's own diagnostic.
POST {{baseurl}}/tools/sed
{"arguments": {"script": "s/unterminated", "input": "x\n"},
 "metadata": {"scratch-dir": "{{test_dir}}"}}
HTTP 422
[Asserts]
jsonpath "$.error.kind" == "std:invalid-args"
