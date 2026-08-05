GET {{baseurl}}/info
HTTP 200
[Asserts]
jsonpath "$.std.name" == "sed"
jsonpath "$.std.capabilities['wasi:filesystem']" exists
