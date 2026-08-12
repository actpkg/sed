import json
import subprocess


def test_manifest_reports_name_and_capabilities(act_command, wasm_path):
    out = subprocess.run(
        [*act_command, "inspect", "component-manifest", str(wasm_path)],
        capture_output=True, text=True, check=True,
    ).stdout
    manifest = json.loads(out)
    assert manifest["std"]["name"] == "sed"
    assert "wasi:filesystem" in manifest["std"]["capabilities"]
