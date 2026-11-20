# Publishing

The development history of Webox holds secrets, personal data and files from the official app, so it never goes public, not even rewritten. The public repository [PianoNic/Webox](https://github.com/PianoNic/Webox) starts from one fresh commit made from the cleaned tree.

## What has to be true first

- `secrets.toml` exists only locally (it is in `.gitignore`); the source has no client id, client secret or API key.
- `assets/cisco/` holds the files from the official app and is ignored; nothing else under `assets/` comes from Cisco ([cisco-assets](cisco-assets.md)).
- Sample data, tests, comments and screenshots use made-up people and places.
- `cargo build` and `cargo test` pass.

## The deny-list

`scripts/check-company-data.ps1` (and `.sh`) check every tracked and untracked-but-not-ignored file, binaries and file names included, against a deny-list of real names, company names, hostnames, local paths and secret prefixes. The list itself is private, so it lives **outside** the repository and its path is passed in:

```text
# category<TAB>regex, case-insensitive; a category ending in ! is case-sensitive
person	firstname
company	companyname
path	Users[\\/]+yourlogin\b
secret	firstcharsofthesecret
```

The script also always looks for co-author trailers and tool attribution. It prints counts per category, never the matches.

```powershell
pwsh scripts/check-company-data.ps1 -DenyList ..\private\denylist.txt
```

## Make the fresh repository

```powershell
pwsh scripts/publish-fresh.ps1 -Destination ..\Webox-public -DenyList ..\private\denylist.txt
```

The script refuses uncommitted changes, runs the check, exports `HEAD` with `git archive` into the empty destination, runs the check again there, and makes one commit authored and committed by `PianoNic <79938743+PianoNic@users.noreply.github.com>` with hooks turned off. It then verifies that the commit is the only one and carries no attribution. It never adds a remote and never pushes.

## Push it (by hand)

Look through the new folder first: `git -C ..\Webox-public show --stat`. Then create the empty repository on GitHub (no README, no licence, so there is nothing to merge) and push:

```powershell
cd ..\Webox-public
git remote add origin https://github.com/PianoNic/Webox.git
git push -u origin main
```

After the first push, create the labels the release drafter uses: `feature`, `enhancement`, `refactor`, `bug`, `documentation`, `CI/CD`, `dependencies`, `skip-changelog`, `major`, `minor`, `patch`, `breaking`, `bugfix`, `hotfix`, `security`.

## Later updates: one commit per pull request

The development history stays private, so the public repository grows by one commit per update. `scripts/publish-update.ps1` makes that commit in your existing clone:

```powershell
pwsh scripts/publish-update.ps1 -Clone C:\coding\Webox -DenyList ..\publish-private -Message 'feat: added the app updater and the Windows installer'
```

It refuses uncommitted changes (here and in the clone), fetches `origin` in the clone, creates `release/<date>` (or `-Branch`) from `origin/main`, replaces every tracked file with `HEAD` of this repository (the clone's `.git` and its ignored files stay), and commits it as `PianoNic <79938743+PianoNic@users.noreply.github.com>` with hooks off. It then verifies that the result is exactly one commit on top of `origin/main` and that neither the diff, the message nor the summary carries attribution or private data. `-FullScan` also runs the guard over both trees; it is off by default because every commit here already passed the pre-commit hook.

The pull-request body is `-Summary`, or else the `feat` / `fix` / `perf` / `refactor` / `docs` titles of the first-parent history since the last export. The script records the exported commit in `publish-private\last-export.txt` (never in the public repository) and writes the body to `publish-private\pr-body.md`. If anything fails, the clone goes back to where it was. It never pushes; it prints the two commands:

```powershell
git -C C:\coding\Webox push -u origin release/2026-11-20
gh pr create --repo PianoNic/Webox --base main --head release/2026-11-20 --title "feat: ..." --body-file ..\publish-private\pr-body.md
```

The pull-request title is what the changelog shows and what picks the label: `feat:` → `feature` (minor bump), `fix:` → `bug` (patch), `docs:` → `documentation`, `ci:` / `build:` → `CI/CD`, a `!` before the colon → `breaking` (major). Merge it with "Squash and merge" so `main` keeps one commit per update.

## Releases

1. Every merge into `main` updates the draft release (`.github/workflows/release-drafter.yml`). Its version is the last tag plus the largest bump among the merged pull requests (patch when none of them carries a bumping label).
2. Publishing the draft runs `.github/workflows/release.yml`: the CI gates, then `scripts/bump-version.sh` sets `Cargo.toml` and the `webox` entry of `Cargo.lock` to the tag and pushes that to `main` (`[skip ci]`), then the builds attach their files to the release:

| File | What |
|---|---|
| `Webox-Setup-<version>-x64.exe` | Windows installer (Inno Setup, `packaging/windows/webox.iss`) |
| `webox-<version>-windows-x64.zip` | Windows, portable; what the in-app updater downloads |
| `webox-<version>-linux-<arch>.tar.gz` | Linux tarball (`packaging/linux/make-tarball.sh`) |
| `<file>.sha256` | `<hex>  <file>` for each of them; the updater refuses a zip without a matching one |

The builds are not code-signed, so Windows SmartScreen warns on the first start of the installer ("More info" → "Run anyway"). No plugins are bundled: they come from the plugin repository.

## The Windows installer

Per-user by default: `%LOCALAPPDATA%\Programs\Webox`, no admin rights, so the in-app updater can replace `webox.exe` later. It adds a Start menu entry, optionally a desktop shortcut and "start with Windows" (the same `Run` value the app's own setting writes), and an entry under Apps (publisher PianoNic). It asks to close a running Webox first (`AppMutex`). Uninstalling keeps `%APPDATA%\WebOx` (settings, sign-in, installed plugins) unless you answer Yes when asked. Build it locally with Inno Setup 6 or 7:

```powershell
cargo build --release
pwsh packaging/windows/make-ico.ps1 -Out dist/webox.ico
mkdir dist/webox-0.0.0-windows-x64; copy target/release/webox.exe, LICENSE, THIRD_PARTY_NOTICES.md, README.md dist/webox-0.0.0-windows-x64/
iscc /DAppVersion=0.0.0 /DSourceDir=dist\webox-0.0.0-windows-x64 /DIconFile=dist\webox.ico /Odist packaging\windows\webox.iss
```

## The in-app updater

"Auf Updates prüfen" and a background check once a day (Settings > Allgemein > Updates) ask `api.github.com/repos/PianoNic/Webox/releases/latest` without signing in and with a generic user agent. Drafts and pre-releases are ignored. A newer version shows a green button in the rail above the settings gear. On Windows "Jetzt aktualisieren" downloads the zip, checks it against its `.sha256`, unpacks `webox.exe` next to the running one, starts it once with `--version`, renames the running file to `webox.exe.old`, moves the new one in and restarts (`--updated`; the new process deletes the `.old` file). Any failure puts the old file back. Only `webox.exe` changes. Nothing is checked, installed or restarted during a call. On Linux the dialog only links to the release: packages update through Nix or the distribution.