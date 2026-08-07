# Releasing rhythr

The steps that have to happen, in the order they have to happen, and the ones
that were forgotten before.

Run `scripts/preflight.sh` first and last. It is not a substitute for this
list, but every check in it exists because something on this list was missed
once.

## Before you tag

1. **Run the checks.**

   ```
   cargo test
   cargo clippy
   scripts/preflight.sh
   ```

   Preflight will fail on the AUR version until step 7, which is expected and
   is the point of running it again at the end.

2. **Refresh the dependency attribution** if anything in `Cargo.lock` moved:

   ```
   scripts/gen-crate-notices.sh
   ```

   `crates/gui/LICENSE.txt` and `crates/gui/THIRD-PARTY-NOTICES.txt` are copies
   of the repository originals, because the bundler can only ship files under
   `crates/gui`. Preflight compares them; if it complains, copy them again.

3. **Take the testing-only controls out of the UI.** Anything that changes what
   a render produces and exists for diagnosis belongs on the CLI, not in
   Advanced. The dry-run switch was one checkbox away from shipping, and it
   makes "Render video" write no file at all while persisting like a
   preference.

4. **Shorten the finished-render message.** The full diagnostic block (which
   replay, which transport, which render since start) is right for a test build
   and noise in a release. The output path, the format and the time are enough;
   the rest belongs in "Save diagnostics".

5. **Read the changelog as a user.** 0.6.0 in particular changes the DEFAULT
   look — camera, approach rate, note shape, colours, hit window. Anyone who
   rendered on an older version without their own config gets visibly different
   videos, and that belongs at the top of the notes as "what changes in your
   renders", not scattered through a list of fixes.

6. **Check the documentation still describes the software.** `docs/USER-GUIDE.md`
   and `README.md` both outlived behaviour changes before.

## Tagging

7. **Bump the versions together.** The workspace `Cargo.toml` and
   `crates/gui/tauri.conf.json` carry the version; every crate including the
   GUI inherits it with `version.workspace = true`. Then:

   ```
   scripts/preflight.sh <new-version>
   ```

8. **Change `## Unreleased` in CHANGELOG.md** to the version and the date.

9. Tag and push, and let the release build produce the artefacts.

## After the artefacts exist

10. **The AUR package comes last, and only now.** It repackages the published
    `.deb`, so its checksums cannot be computed before that file exists.
    Bumping `pkgver` early produces a PKGBUILD with a new version and old
    hashes, which is worse than an obviously stale one.

    ```
    cd packaging/aur
    # bump pkgver, then:
    updpkgsums
    makepkg --printsrcinfo > .SRCINFO
    ```

11. **Verify one artefact per format actually contains the licensing.** The deb
    and rpm should have `/usr/share/doc/rhythr/copyright`; the installer should
    show a license page; all four should carry `LICENSE.txt`,
    `THIRD-PARTY-NOTICES.txt` and `THIRD-PARTY-CRATES.txt` next to the binary.

12. **Run `scripts/preflight.sh` once more.** It should pass clean.
