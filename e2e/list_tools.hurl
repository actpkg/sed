POST {{baseurl}}/tools
{}
HTTP 200
[Asserts]
jsonpath "$.tools" count == 2
jsonpath "$.tools[*].name" contains "sed"
jsonpath "$.tools[*].name" contains "sed_files"
