#!/usr/bin/env python3
"""Generate a CycloneDX 1.5 SBOM for the LUMYX workspace from `cargo metadata`.

Usage: python3 scripts/generate_sbom.py [--out docs/sbom/sbom.cdx.json]

No external Python deps; no network. The SBOM lists every resolved crate with
version, license, and purl — the input cosign/attestation tooling expects.
"""
import json
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path


def main() -> int:
    out_path = Path("docs/sbom/sbom.cdx.json")
    if "--out" in sys.argv:
        out_path = Path(sys.argv[sys.argv.index("--out") + 1])

    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            text=True,
        )
    )
    workspace_ids = set(meta["workspace_members"])

    components = []
    for pkg in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
        is_local = pkg["id"] in workspace_ids
        components.append(
            {
                "type": "library",
                "bom-ref": f"pkg:cargo/{pkg['name']}@{pkg['version']}",
                "name": pkg["name"],
                "version": pkg["version"],
                "purl": f"pkg:cargo/{pkg['name']}@{pkg['version']}",
                "licenses": (
                    [{"license": {"id": pkg["license"]}}] if pkg.get("license") else []
                ),
                "scope": "required",
                "properties": [
                    {"name": "lumyx:origin", "value": "workspace" if is_local else "crates.io"}
                ],
            }
        )

    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{uuid.uuid4()}",
        "version": 1,
        "metadata": {
            "timestamp": datetime.now(timezone.utc).isoformat(),
            "component": {
                "type": "application",
                "name": "lumyx-led-platform",
                "version": "0.1.0",
            },
            "tools": [{"name": "generate_sbom.py", "version": "1"}],
        },
        "components": components,
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(sbom, indent=2) + "\n")
    local = sum(1 for c in components if c["properties"][0]["value"] == "workspace")
    print(f"SBOM: {len(components)} components ({local} workspace, "
          f"{len(components) - local} external) -> {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
