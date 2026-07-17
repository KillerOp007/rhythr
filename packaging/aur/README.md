# AUR package (rhythr-bin)

Repackages the release `.deb` for Arch-based distros (Arch, CachyOS,
EndeavourOS, Manjaro). Nothing here is published automatically — the AUR
side is a separate git repository that has to be pushed by hand.

## Publishing (first time)

1. Create an AUR account at https://aur.archlinux.org and add an SSH key.
2. `git clone ssh://aur@aur.archlinux.org/rhythr-bin.git`
3. Copy `PKGBUILD` in, fill the real `sha256sums` (run `updpkgsums`
   on an Arch box/container, or paste the SHA-256 values from the
   GitHub release notes), then regenerate the metadata:
   `makepkg --printsrcinfo > .SRCINFO`
4. Commit `PKGBUILD` + `.SRCINFO` and push — that IS the release.

## Every release after that

Bump `pkgver`, reset `pkgrel=1`, refresh `sha256sums`, regenerate
`.SRCINFO`, push.

Users then install with an AUR helper (`yay -S rhythr-bin` /
`paru -S rhythr-bin`) and get updates like any other package.
