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

After the first push:

1. Create the labels the release drafter uses: `feature`, `enhancement`, `refactor`, `bug`, `documentation`, `CI/CD`, `dependencies`, `skip-changelog`, `major`, `minor`, `patch`.
2. Publish the first release from the draft. The release workflow bumps `Cargo.toml` to the tag and attaches the Windows zip and the Linux tarball. The release builds contain no client credentials and no files of the official app (see the README's Configuration section).

Later work happens in the public repository. The private one stays as the archive of the development history.
