# Releasing the native preview

The native app (this folder) ships as **TermDrop Native Preview**, a separate
app from the Tauri one. This page is how to cut a preview, what it produces,
and what must never happen. The Tauri release is `../.github/workflows/release.yml`
and the `CHANGELOG.md` at the top of the repo; nothing here touches either.

## What the preview is

| | Tauri app | Native preview |
|---|---|---|
| Name | TermDrop | TermDrop Native Preview |
| Identifier | `com.termdrop` | `com.termdrop.native-preview` |
| Version line | `src-tauri/Cargo.toml` (0.5.x) | `native/Cargo.toml` `[workspace.package]` (0.6.0-preview.N) |
| Tag | `v*` | `native-v*` |
| Workflow | `release.yml` | `native-release.yml` |
| GitHub release | latest, with `latest.json` | **prerelease**, `make_latest: false`, no `latest.json` |
| macOS | `TermDrop.app` | `TermDrop Native Preview.app` (in a `.dmg`) |
| Windows | MSI, per machine | NSIS `.exe`, per user, `%LOCALAPPDATA%\TermDrop Native Preview` |
| Linux | AppImage, `.deb` package `termdrop` | AppImage, `.deb` package `termdrop-native-preview` |
| Auto-update | yes | no |
| Single-instance key | Tauri's plugin, `com.termdrop` | `com.termdrop.native-preview` (`app/src/single_instance.rs`) |

Both apps open the same `termdrop.db`, keyring service and `known_hosts`, on
purpose: the preview is for trying the native app on real hosts. Both can run
at once (the core opens SQLite with a 5 s busy timeout, so neither fails on the
other's lock), but neither notices the other's edits, so the release notes say
to use one at a time. The window's size and position are the native app's own
setting (`native-window-state`).

### Version scheme

`MAJOR.MINOR.PATCH-preview.N`, tagged `native-vMAJOR.MINOR.PATCH-preview.N`.
The first is `0.6.0-preview.1`: the next minor after the Tauri app's 0.5.x,
since the native app is meant to become that line, and a prerelease suffix so
it can never be mistaken for, or sort above, a real 0.6.0.

Each installer accepts it:

- **NSIS** takes the string as is for display and derives the numeric
  `VIProductVersion` (`0.6.0.0`) itself. **WiX/MSI is not used**: it needs a
  purely numeric version, and it is also what the Tauri app installs with.
- **deb**: `0.6.0-preview.1` reads as upstream `0.6.0`, revision `preview.1`,
  so previews sort in order (`preview.2` > `preview.1`, `preview.10` > `preview.9`).
  The package name differs from the Tauri one, so the two never replace each other.
- **macOS**: `CFBundleShortVersionString` carries the string; only the App
  Store insists on digits.

## Cutting a preview

1. On the branch you are releasing from, with native CI green on all three
   platforms (`native-ci.yml`):
   ```bash
   cd native
   # edit Cargo.toml: [workspace.package] version = "0.6.0-preview.2"
   cargo check --offline --workspace   # updates Cargo.lock's own versions
   git commit -am "chore(native): 0.6.0-preview.2"
   ```
2. Optional but recommended: run the packaging without a tag first (below).
3. Tag and push the tag:
   ```bash
   git tag native-v0.6.0-preview.2
   git push origin native-v0.6.0-preview.2
   ```
   The workflow fails at once if the tag and the workspace version differ.

`native-release.yml` then, on macos-14 (Apple silicon), windows-2022 and
ubuntu-22.04:

- builds `termdrop-native` in release mode and packages it with
  `cargo-packager` 0.11.8 (config: `[package.metadata.packager]` in
  `app/Cargo.toml`): `.app` + `.dmg`, NSIS `-setup.exe`, `.deb` + `.AppImage`;
- checks `mongodump` and `mongorestore` are inside each package, where
  `termdrop-core`'s `resolve_mongo_tool` looks for them:

  | Package | Location |
  |---|---|
  | macOS `.app` | `Contents/Resources/` |
  | Windows NSIS | the install folder, beside `termdrop-native.exe` |
  | `.deb`, AppImage | `usr/lib/termdrop-native/` |

- uploads the packages as workflow artifacts, then creates one GitHub
  **prerelease** for the tag, titled "TermDrop Native Preview <version>", with
  `make_latest: false` and the notes in the workflow (preview beside the Tauri
  app, shared data so one app at a time, no auto-update, Tunnels not ported,
  unsigned so Gatekeeper/SmartScreen warn).

## Testing the packaging without a release

Push the commit to test to the branch `native/package-test`
(`git push -f origin HEAD:native/package-test`). While this workflow is not on
master, GitHub offers no *Run workflow* button for it, so that push is the way
in; once it is on master, Actions → **Native Preview Release** → *Run workflow*
on any branch does the same. Either way it runs the same build, package and
checks, uploads the packages as artifacts named
`termdrop-native-preview-<version>-<platform>`, and creates no release (the
release job runs only for `native-v*` tags).

Locally, on Linux (from `native/app`; the AppImage step downloads linuxdeploy
on first use):

```bash
cargo install cargo-packager --version 0.11.8 --locked
cargo packager --release --formats deb,appimage    # writes to native/target/packager/
dpkg-deb -c ../target/packager/*.deb | grep mongo
```

The packager config assumes the default target dir (`native/target`): its
resources are the tools `build.rs` copies to `native/target/release/`.

## Never

- **Never tag a native build `v*`.** That runs the Tauri `release.yml`, which
  would publish a Tauri build under that tag and move `latest.json`, the file
  every installed TermDrop polls for updates.
- **Never mark a preview release "latest"**, in the workflow or by hand on
  GitHub. The Tauri updater fetches `releases/latest/download/latest.json`;
  a preview as latest has no `latest.json`, which breaks updates for every
  Tauri user.
- **Never add a `latest.json`, updater signature or signing key** to the
  preview. The switch from Tauri to native (plan, Phase 8) is a separate,
  deliberate release with its own constraints: an MSI with the Tauri upgrade
  code, the same minisign key, and the `com.termdrop` identity.
- **Never edit `release.yml`** for the preview, and do not reuse its
  `TAURI_SIGNING_PRIVATE_KEY` secret here.
- Do not change the preview's product name or identifier between previews:
  the identifier keys the macOS bundle and the single-instance lock, and the
  product name keys the Windows install folder and uninstall entry, so a
  rename installs a second copy.
