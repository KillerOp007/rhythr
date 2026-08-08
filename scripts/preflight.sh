#!/usr/bin/env bash
# Checks the things that were wrong before a release rather than after it.
#
# Every check here exists because it was actually broken once: the AUR package
# sat at 0.5.0 with 0.5.0 checksums while the tree was 0.6.0, the GUI crate
# pinned its own version instead of the workspace's, four package formats
# shipped without the project's own license in them, and a debug control that
# makes a render write no file was one checkbox away from the public.
#
#   scripts/preflight.sh          check against the workspace version
#   scripts/preflight.sh 0.6.0    check against a version you are about to tag
#
# Exits non-zero if anything is wrong. It changes nothing.
set -uo pipefail
cd "$(dirname "$0")/.."

FAIL=0
note() { printf '  %-6s %s\n' "$1" "$2"; }
ok()   { note "ok" "$1"; }
bad()  { note "FAIL" "$1"; FAIL=1; }
warn() { note "warn" "$1"; }

WS_VERSION=$(python3 - <<'PY'
import re
t = open("Cargo.toml").read()
m = re.search(r'^\s*version\s*=\s*"([^"]+)"', t, re.M)
print(m.group(1) if m else "")
PY
)
WANT="${1:-$WS_VERSION}"
echo "preflight for version $WANT"
echo

echo "versions agree"
[ -n "$WS_VERSION" ] && ok "workspace Cargo.toml: $WS_VERSION" || bad "workspace version not found"
[ "$WS_VERSION" = "$WANT" ] || bad "workspace is $WS_VERSION, expected $WANT"

if grep -qE '^\s*version\s*=\s*"' crates/gui/Cargo.toml; then
  bad "crates/gui/Cargo.toml pins its own version instead of version.workspace = true"
else
  ok "crates/gui follows the workspace version"
fi

TAURI_V=$(python3 -c "import json;print(json.load(open('crates/gui/tauri.conf.json')).get('version',''))")
[ "$TAURI_V" = "$WANT" ] && ok "tauri.conf.json: $TAURI_V" || bad "tauri.conf.json is $TAURI_V, expected $WANT"

AUR_V=$(grep -oP '^pkgver=\K.*' packaging/aur/PKGBUILD 2>/dev/null || echo "")
[ "$AUR_V" = "$WANT" ] && ok "AUR PKGBUILD: $AUR_V" || bad "AUR PKGBUILD is ${AUR_V:-missing}, expected $WANT (and its sha256sums need refreshing)"

echo
echo "licensing ships with the binaries"
python3 - "$WANT" <<'PY'
import json, sys
b = json.load(open("crates/gui/tauri.conf.json"))["bundle"]
res = b.get("resources", [])
problems = []
if b.get("license") != "MIT":
    problems.append("bundle.license is not MIT")
if not b.get("licenseFile"):
    problems.append("bundle.licenseFile is unset, so NSIS shows no license page")
for f in ("LICENSE.txt", "THIRD-PARTY-NOTICES.txt", "THIRD-PARTY-CRATES.txt"):
    if f not in res:
        problems.append(f"{f} is not in bundle.resources")
for fmt in ("deb", "rpm"):
    files = b.get("linux", {}).get(fmt, {}).get("files", {})
    if "/usr/share/doc/rhythr/copyright" not in files:
        problems.append(f"{fmt} installs no copyright file")
for p in problems:
    print(f"  FAIL   {p}")
print("  ok     bundle licensing complete" if not problems else "")
sys.exit(1 if problems else 0)
PY
[ $? -eq 0 ] || FAIL=1

# The platform configs REPLACE bundle.resources rather than adding to it, so a
# file listed only in tauri.conf.json silently misses that platform. That is
# exactly how the Windows installer went out without the license while the
# AppImage had it.
for pc in crates/gui/tauri.windows.conf.json crates/gui/tauri.macos.conf.json crates/gui/tauri.linux.conf.json; do
  [ -f "$pc" ] || continue
  MISSING=$(python3 -c '
import json, sys
res = json.load(open(sys.argv[1])).get("bundle", {}).get("resources")
if res is not None:
    want = ("LICENSE.txt", "THIRD-PARTY-NOTICES.txt", "THIRD-PARTY-CRATES.txt")
    print(" ".join(f for f in want if f not in res))
' "$pc")
  if [ -n "$MISSING" ]; then
    bad "$(basename "$pc") overrides bundle.resources and drops: $MISSING"
  else
    ok "$(basename "$pc") keeps the licensing"
  fi
done

# The AppImage is the one Linux format that cannot rely on a package manager
# pulling ffmpeg in, so it carries its own copy. deb and rpm declare it as a
# dependency instead and must NOT carry one.
python3 -c '
import json, sys
b = json.load(open("crates/gui/tauri.conf.json"))["bundle"]
files = b.get("linux", {}).get("appimage", {}).get("files", {})
if not any(k.endswith("/ffmpeg") for k in files):
    print("  FAIL   the AppImage bundles no ffmpeg, so it cannot render on a machine without one")
    sys.exit(1)
print("  ok     the AppImage carries its own ffmpeg")
' || FAIL=1

# Tauri only reads tauri.<platform>.conf.json for linux, windows and macos. A
# file named after a BUNDLE FORMAT is silently ignored, which is how the
# AppImage lost the ffmpeg that resolve_ffmpeg documents as its fallback.
for stray in crates/gui/tauri.appimage.conf.json crates/gui/tauri.deb.conf.json crates/gui/tauri.rpm.conf.json crates/gui/tauri.nsis.conf.json; do
  [ -f "$stray" ] && warn "$(basename "$stray") is named after a bundle format, not a platform, so Tauri never reads it"
done

for pair in "LICENSE:crates/gui/LICENSE.txt" "THIRD-PARTY-NOTICES.md:crates/gui/THIRD-PARTY-NOTICES.txt"; do
  src="${pair%%:*}"; dst="${pair##*:}"
  if [ ! -f "$dst" ]; then
    bad "$dst is missing (copy it from $src)"
  elif ! cmp -s "$src" "$dst"; then
    bad "$dst has drifted from $src (copy it again)"
  else
    ok "$dst matches $src"
  fi
done

if [ -f crates/gui/THIRD-PARTY-CRATES.txt ]; then
  CUR=$(grep -c '^    license:' crates/gui/THIRD-PARTY-CRATES.txt || echo 0)
  LOCKED=$(grep -c '^name = ' Cargo.lock || echo 0)
  if [ "$CUR" -lt $((LOCKED - 20)) ]; then
    bad "THIRD-PARTY-CRATES.txt lists $CUR crates against $LOCKED in Cargo.lock — run scripts/gen-crate-notices.sh"
  else
    ok "crate attribution present ($CUR crates)"
  fi
else
  bad "crates/gui/THIRD-PARTY-CRATES.txt is missing — run scripts/gen-crate-notices.sh"
fi

echo
echo "nothing for testing only is exposed"
if grep -q 'id="set-dryrun"' crates/gui/ui/index.html; then
  bad "the dry-run control is in the UI: a render would write no file, and the setting persists"
else
  ok "no dry-run control in the UI"
fi
if grep -q 'render #\|replay restored at startup\|opened by hand' crates/gui/src/main.rs; then
  warn "the finished-render message still carries the full diagnostic block (fine for a test build, noise for a release)"
else
  ok "finished-render message is release-shaped"
fi

echo
echo "documentation is not describing the old behaviour"
if grep -qi "CRF.*lower is better\|lower = better" docs/*.md README.md 2>/dev/null; then
  bad "the docs still describe quality as a CRF where lower is better"
else
  ok "quality is documented the way it now works"
fi
if grep -q '^## Unreleased' CHANGELOG.md; then
  warn "CHANGELOG.md still says Unreleased (correct until you tag, wrong after)"
else
  ok "changelog has a released heading"
fi

echo
if [ "$FAIL" -eq 0 ]; then
  echo "preflight passed"
else
  echo "preflight FAILED"
fi
exit $FAIL
