"""Create the bundled offline lookup from an upstream Ludusavi manifest.

Usage: python scripts/import-ludusavi.py path/to/manifest.yaml
Requires PyYAML. Preserve the accompanying upstream license and attribution.
"""
import json
import sys
from pathlib import Path

import yaml

with open(sys.argv[1], encoding="utf-8") as source:
    games = yaml.load(source, Loader=yaml.CSafeLoader)
records = []
for name, game in games.items():
    if not isinstance(game, dict) or not game.get("files"):
        continue
    steam = game.get("steam") or {}
    ids = [steam.get("id"), *((game.get("id") or {}).get("steamExtra") or [])]
    ids = [str(i) for i in ids if isinstance(i, int) and i > 0]
    files = []
    for path, detail in game["files"].items():
        detail = detail or {}
        tags = detail.get("tags") or []
        if tags and "save" not in tags:
            continue
        when = detail.get("when") or []
        # Store-specific rules for other launchers cannot be safely applied to Steam.
        if when and not any(w.get("store") in (None, "steam") for w in when):
            continue
        files.append([path, [w.get("os") for w in when if w.get("os")]])
    if files:
        records.append({"name": name, "ids": ids, "aliases": list((game.get("installDir") or {}).keys()), "files": files})
target = Path(__file__).resolve().parents[1] / "crates/core/manifests/ludusavi.json"
target.write_text(json.dumps(records, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
print(f"Wrote {len(records)} games to {target}")
