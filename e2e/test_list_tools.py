async def test_lists_both_tools(client):
    tools = await client.list_tools()
    names = [t.name for t in tools]
    assert len(tools) == 2
    assert "sed" in names
    assert "sed_files" in names
