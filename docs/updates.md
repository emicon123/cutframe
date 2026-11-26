# Updates: release key, publishing and testing

Cutframe updates itself with `tauri-plugin-updater` (F0.12). An installed copy reads

```
https://github.com/emicon123/cutframe/releases/latest/download/latest.json
```

(`plugins.updater.endpoints` in `src-tauri/tauri.conf.json`). That file names the newest NSIS
installer and its minisign signature. The app downloads the installer, checks the signature against
`plugins.updater.pubkey` and runs it in `passive` mode. The install is per-user (NSIS
`currentUser`), so there is no admin prompt. The installer and `.exe` are not Authenticode-signed
(private use), so SmartScreen may still warn on the very first manual install.

How the pieces fit:

| Piece | Where it lives |
|---|---|
| Public key | `plugins.updater.pubkey` in `src-tauri/tauri.conf.json` (committed, not secret) |
| Private key + password | GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (never in the repo) |
| Updater artifacts | turned on only in CI: `release.yml` passes `--config src-tauri/tauri.updater.conf.json` (`bundle.createUpdaterArtifacts: true`), so a local `pnpm tauri build` works without the private key |
| `latest.json` | written by `release.yml` and uploaded to the draft release next to `*-setup.exe` and `*-setup.exe.sig` |

## One-time setup (the repository owner)

Do these steps in order. Steps 2 to 4 must be done before the first release that installed copies
should update from: a build made with the placeholder key never checks for updates.

1. **Make the repository public.** On GitHub go to *Settings → General → Danger Zone → Change
   visibility → Make public*. Release downloads from a private repository need a token, and the
   installed app has none.

2. **Generate the updater key pair, outside the repository**, on your Windows machine:

   ```powershell
   pnpm tauri signer generate -w "$env:USERPROFILE\.tauri\cutframe-updater.key"
   ```

   Enter a password when asked (a non-empty one: the release workflow requires it). This writes
   the private key to `cutframe-updater.key` and the public key to `cutframe-updater.key.pub`.
   **Back up the private key file and the password somewhere outside GitHub** (e.g. a password
   manager). If they are lost, or the key is ever replaced, every installed copy stops accepting
   updates and has to be reinstalled by hand.

3. **Put the public key into the app config.** Open `cutframe-updater.key.pub`, copy its whole
   contents (one long base64 line) and paste it as the value of `plugins.updater.pubkey` in
   `src-tauri/tauri.conf.json`, replacing the `PLACEHOLDER: ...` text. Commit and push that change.

4. **Add the two Actions secrets.** On GitHub go to *Settings → Secrets and variables → Actions →
   New repository secret* and add:

   | Name | Value |
   |---|---|
   | `TAURI_SIGNING_PRIVATE_KEY` | the whole contents of `cutframe-updater.key` (not the path) |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password from step 2 |

   Never paste the private key into a file in the repository, an issue, a chat or a commit.

The release workflow checks steps 3 and 4 before it builds. It stops with a clear error if a secret
is missing or the public key is still the placeholder.

## Release flow

1. Bump `version` in the workspace `Cargo.toml` (semver, e.g. `0.2.0`), run `pnpm check`, commit and
   push.
2. Tag the commit and push the tag. An annotated tag's message becomes the release notes that the
   app shows in its update prompt (a lightweight tag gets the default text "Cutframe vX.Y.Z"):

   ```powershell
   git tag -a v0.2.0 -m "Faster thumbnails, new crop handles"
   git push origin v0.2.0
   ```

   The tag must match the `Cargo.toml` version, or the workflow fails.
3. `.github/workflows/release.yml` builds on `windows-latest` and creates a **draft** release
   "Cutframe v0.2.0" with `Cutframe_0.2.0_x64-setup.exe`, `Cutframe_0.2.0_x64-setup.exe.sig` and
   `latest.json`.
4. Optionally download the draft's `-setup.exe` and try it.
5. **Publish the draft**: on GitHub open *Releases → the draft → Edit → Publish release* (or
   `gh release edit v0.2.0 --draft=false`). Do not tick *Set as a pre-release*: GitHub's
   `releases/latest` ignores drafts and pre-releases, so only a published, full release reaches
   installed copies. From this moment every installed copy is offered the update.

### Why releases stay drafts

Publishing is what rolls an update out to every installed copy, and it cannot really be taken back
(a bad update that is already installed has to be fixed by a newer one). Keeping the draft step
gives you one look at the build before that happens, and costs one click. The workflow therefore
never publishes on its own. If you later want one-step releases, `releaseDraft: false` on the
tauri-action step in `release.yml` publishes directly, but the release would then become "latest"
before `latest.json` is uploaded, so the upload step would have to move ahead of publishing.

`notes`, `version` and `pub_date` in `latest.json` are fixed at build time. Editing the release
text on GitHub before publishing changes the release page, not the text in the app's prompt.

## Testing an update end to end

Needs two releases that both carry the real public key (step 3). Builds from before step 3 never
check for updates.

1. Release `vA` (e.g. `v0.2.0`) as above and publish it.
2. On a Windows 10/11 machine, as a normal (non-admin) user, download `Cutframe_A_x64-setup.exe`
   from the release page and install it. It installs into `%LOCALAPPDATA%\Cutframe` with no UAC
   prompt. Start the app and check that it reports no update.
3. Bump the version, tag and release `vB` (e.g. `v0.2.1`), and publish the draft.
4. Check that `https://github.com/emicon123/cutframe/releases/latest/download/latest.json` now shows
   `"version": "B"` and a `windows-x86_64` entry whose `url` downloads the B installer.
5. Start (or restart) the installed A. It offers version B with the tag notes. Accept: it downloads
   with progress, then restarts into the installer, which runs passively with **no admin prompt**
   and starts the app again.
6. Check that the app now reports version B and that projects, preferences and recent files are
   still there (they live in the app data folders, not next to the `.exe`).

Things that can go wrong:

| Symptom | Likely cause |
|---|---|
| Workflow fails in "Updater preflight" | a secret is missing, or `pubkey` is still the placeholder |
| App never offers the update | the newer release is still a draft or a pre-release; the installed build has the placeholder key; or the version is not higher |
| App reports a signature error | the public key in the installed build does not match the private key in the secrets (key replaced) |
| Download fails with 404 | the repository is still private |

## Developing against the updater

Debug builds (`pnpm dev`) check for updates only with `CUTFRAME_UPDATER=1` set (see
`src-tauri/src/updater.rs`), so a dev build never offers to replace the installed app.
