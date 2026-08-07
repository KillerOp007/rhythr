#!/usr/bin/env bash
# Regenerates crates/gui/THIRD-PARTY-CRATES.txt, the dependency attribution
# that ships next to the app.
#
# rhythr is MIT and nearly all of its dependencies are MIT or Apache-2.0.
# Both licenses require the notice to travel with the binary, and shipping
# four package formats with none of it in them is the gap this closes.
#
# Run it whenever dependencies change, and before a release.
set -euo pipefail
cd "$(dirname "$0")/.."

META=$(mktemp)
trap 'rm -f "$META"' EXIT
cargo metadata --format-version 1 --all-features > "$META"

python3 - "$META" crates/gui/THIRD-PARTY-CRATES.txt <<'PY'
import json, sys, collections

meta_path, out_path = sys.argv[1], sys.argv[2]
with open(meta_path) as f:
    meta = json.load(f)

deps = [p for p in meta["packages"] if not p["name"].startswith("rhythia-")]
deps.sort(key=lambda p: (p["name"].lower(), p["version"]))

by_license = collections.Counter((p.get("license") or "see repository") for p in deps)

lines = [
    "Third-party Rust crates in rhythr",
    "=================================",
    "",
    "rhythr is MIT licensed. It is built from the crates listed below, whose",
    "licenses require their notices to travel with the binary.",
    "",
    "Full license texts are distributed with each crate's source and can be",
    "read at the repository listed beside it, or fetched with `cargo vendor`",
    "against this project's Cargo.lock.",
    "",
    f"{len(deps)} crates, by license:",
]
for lic, n in sorted(by_license.items(), key=lambda kv: (-kv[1], kv[0])):
    lines.append(f"  {n:5d}  {lic}")
lines += ["", "-" * 70, ""]
for p in deps:
    lines.append(f"{p['name']} {p['version']}")
    lines.append(f"    license: {p.get('license') or 'see repository'}")
    if p.get("repository"):
        lines.append(f"    source:  {p['repository']}")
    lines.append("")

with open(out_path, "w") as f:
    f.write("\n".join(lines))
print(f"wrote {out_path}: {len(deps)} crates")
PY
