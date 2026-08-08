#!/usr/bin/env bash
# Publishes packaging/aur/ to the AUR, which is a separate git repository at
# aur.archlinux.org that nothing else pushes to.
#
# Run it AFTER the GitHub release exists: the PKGBUILD carries the checksums
# of the published .deb, so it cannot be written before that file does.
#
#   scripts/push-aur.sh            # push whatever packaging/aur/ says
#   scripts/push-aur.sh --check    # only say whether the AUR is accepting
#
# The AUR goes into maintenance from time to time, during which the web
# interface and the RPC keep answering while every git command is refused
# with "The AUR is down due to maintenance". That is why this checks first
# and says so plainly instead of leaving a git error to interpret.
set -uo pipefail
cd "$(dirname "$0")/.."

REPO=ssh://aur@aur.archlinux.org/rhythr-bin.git
SSH_OPTS="ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=20"

PKGVER=$(grep -oP '^pkgver=\K.*' packaging/aur/PKGBUILD)
SRCVER=$(grep -oP '^\s*pkgver = \K.*' packaging/aur/.SRCINFO)
if [ "$PKGVER" != "$SRCVER" ]; then
  echo "PKGBUILD is $PKGVER and .SRCINFO is $SRCVER: regenerate .SRCINFO" >&2
  exit 1
fi

echo "checking whether the AUR is accepting pushes"
ANSWER=$(timeout 30 ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new \
  aur@aur.archlinux.org list-repos 2>&1)
if grep -qi "maintenance" <<<"$ANSWER"; then
  echo "the AUR is in maintenance right now:"
  echo "  $ANSWER"
  echo "packaging/aur/ is ready for $PKGVER; run this again when it is back."
  exit 2
fi
if ! grep -q "rhythr-bin" <<<"$ANSWER"; then
  echo "the AUR did not list rhythr-bin for this key:" >&2
  echo "  $ANSWER" >&2
  exit 1
fi
echo "  the AUR is up and this key owns rhythr-bin"
[ "${1:-}" = "--check" ] && exit 0

# The published .deb has to match the checksum in the PKGBUILD, or every
# install fails validation. Cheap to verify, and only possible from here.
DEB_URL=$(grep -oP '^\s+"\K[^"]*\.deb' packaging/aur/PKGBUILD | head -1)
DEB_URL=${DEB_URL//\$url/https://github.com/KillerOp007/rhythr}
DEB_URL=${DEB_URL//\$pkgver/$PKGVER}
WANT=$(grep -A 1 '^sha256sums=' packaging/aur/PKGBUILD | head -1 | grep -oP "[0-9a-f]{64}")
echo "verifying the published .deb against the PKGBUILD checksum"
GOT=$(curl -sL --max-time 300 "$DEB_URL" | sha256sum | cut -d' ' -f1)
if [ "$GOT" != "$WANT" ]; then
  echo "the published .deb hashes $GOT, the PKGBUILD expects $WANT" >&2
  echo "(is the release uploaded, and is it the same file that was hashed?)" >&2
  exit 1
fi
echo "  matches"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
GIT_SSH_COMMAND="$SSH_OPTS" git clone --quiet "$REPO" "$TMP/aur" || {
  echo "could not clone $REPO" >&2
  exit 1
}
cp packaging/aur/PKGBUILD packaging/aur/.SRCINFO "$TMP/aur/"
cd "$TMP/aur"
if git diff --quiet; then
  echo "the AUR already has exactly this: nothing to push"
  exit 0
fi
git add PKGBUILD .SRCINFO
git -c user.name="KillerOp007" \
    -c user.email="79337152+KillerOp007@users.noreply.github.com" \
    commit --quiet -m "rhythr-bin $PKGVER"
GIT_SSH_COMMAND="$SSH_OPTS" git push origin master
echo "pushed rhythr-bin $PKGVER to the AUR"
