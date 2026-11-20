# Privacy guard

WebOx is developed against a real Webex account. Nothing from that account (people, spaces, teams,
organisations, ids, hosts) and nothing about the developer's machine may end up in this repository,
and commits carry no tool attribution. `scripts/check-company-data.ps1` enforces that; git hooks run it
on every commit.

## What is checked

| category | rule |
|---|---|
| private deny-list categories (`person`, `person-token`, `email`, `email-local`, `domain`, `company`, `space`, `team`, `uuid`, `machine`, ...) | Regexes from deny-lists kept **outside** the repository. Case-insensitive; umlauts also match their ASCII spellings (ä = ae = a, ö = oe = o, ü = ue = u, ß = ss). |
| `email` | Every e-mail address, except reserved example domains (`example.com/.org/.net` and subdomains), reserved TLDs (`.example .invalid .test .localhost .local .internal .lan`), `*@users.noreply.github.com` and `noreply@` / `no-reply@` / `donotreply@` addresses. |
| `phone` | International numbers (`+41 ...`, `0041 ...`), Swiss/German style `0xx xxx xx xx`, `tel:` URIs. Numbers reserved for fiction pass: NANP `555-0100` to `555-0199` (any area code) and UK `07700 900000` to `07700 900999`; so do obvious fakes ending in five or more zeros or containing `1234567` / `7654321`. |
| `uuid` | Every UUID/GUID, except obviously synthetic ones: each group a single repeated character (`11111111-2222-3333-4444-555555555555`), at most four non-zero hex digits (`00000000-0000-0000-0000-000000000042`), the counting sequence (`01234567-89ab-cdef-0123-456789abcdef`), `12345678-1234-1234-1234-123456789abc`. Anything else that is public (e.g. a Windows interface id) goes into the allowlist. |
| `webex-id` | Base64 Webex ids (`Y2lzY29zcGFyazovL...`) whose embedded UUID is not synthetic. |
| `tenant-host` | Hosts under `webex.com`, `ciscospark.com`, `webexapis.com`, `wbx2.com`, `webex.<cc>` that name a customer site, and personal-room paths (`webex.com/meet/<name>`). Generic service hosts pass (`*.wbx2.com`, `*.prod.infra.webex.com`, `idbroker`, `u2c`, `web`, `help`, ...; placeholder sites `example`, `yoursite`, `acme`, single letters, ...). |
| `user-path` | `C:\Users\<name>`, `/home/<name>`, `/Users/<name>` unless `<name>` is a placeholder (`you`, `user`, `Your Name`, `YOURNAME`, `test`, `alice`, `bob`, one or two single letters such as `a b`, `<name>`, `%USERNAME%`, `Public`, `Default`, ...). |
| `machine` | Windows default machine names (`DESKTOP-XXXXXXX`, `LAPTOP-...`); the real machine name is in the private list. |
| `attribution` | Co-author trailers, "Generated with/by ..." tool lines, the assistant's and its vendor's names (case-sensitive), vendor addresses, statements that an AI tool wrote the content. |
| `image-review` | Every tracked image outside `assets/icons/`, `assets/icon/`, `assets/emoji/` needs a reviewed allowlist entry with its blob hash: a screenshot can show real data that no text rule sees. A changed image needs a new review. |
| `identity` | Commit author / committer e-mail must pass the `email` rule (the owner's public identity `PianoNic <79938743+PianoNic@users.noreply.github.com>` does); names are checked against the deny-lists. |

What is scanned: file contents (text as is; binaries as their printable ASCII/UTF-16 strings, PNG
`tEXt`/`zTXt`/`iTXt` chunks inflated, JPEG EXIF/XMP, PDF streams inflated), file names, commit messages
and identities. Findings print as `category  file:line` only; matched values are never printed.

## Running it

```powershell
pwsh scripts/check-company-data.ps1                                   # working tree incl. untracked, not ignored
pwsh scripts/check-company-data.ps1 -Mode rev -Rev main -Commits      # a commit's tree + every commit message on it
pwsh scripts/check-company-data.ps1 -Mode staged                      # what the pre-commit hook checks
pwsh scripts/check-company-data.ps1 -Context                          # also show each line, every match masked as «category»
pwsh scripts/check-company-data.ps1 -DenyList D:\private\denylist.txt # explicit deny-list(s) instead of git config
```

`scripts/check-company-data.sh` passes its arguments to the PowerShell script (Linux/NixOS:
`nix-shell -p powershell`); without `pwsh` it fails closed.

## The private deny-lists

Lines `category<TAB>regex`; a category ending in `!` is case-sensitive; `#` starts a comment. They are
never committed. The guard reads them from `-DenyList`, else from the repository's local git config
`webox.denylist` (a file, or a folder whose `denylist*.txt` are all read), else from `$env:WEBOX_DENYLIST`
(`;`-separated). Without any list only the generic rules run (and a warning says so).

## Hooks

```powershell
pwsh scripts/install-privacy-hooks.ps1 -DenyList <folder or file outside the repo>
```

installs `pre-commit`, `pre-merge-commit` (staged files) and `commit-msg` (message + identity) into the
repository's common git folder, so they cover every worktree. The guard is copied next to the hooks, so
worktrees on older branches use the current rules; re-run the installer after changing the guard. A
blocked commit lists `category  file:line`. Fix it by replacing the value with a made-up one
(`@example.com`, a synthetic id, an invented name, a `555-01xx` number, `C:\Users\you`).

## False positives: `scripts/privacy-allowlist.txt`

The only way past a finding that is not private data. Tab-separated lines, `#` comments:

```
category<TAB>value<TAB>path glob (optional, default *)<TAB># why
uuid	aac56b...	docs/architecture.md	# public WinTrust action GUID
email	sha256:<hex of the lower-cased match>	src/x.rs	# a deny-list word that is an ordinary word here
image	assets/branding/screenshot.png	<git blob sha1>	# reviewed: sample-mode data only
```

- Generic rules (`email`, `uuid`, `phone`, `tenant-host`, `user-path`, ...) may be allowed by the literal
  value; only add values that are public or invented.
- A deny-list match can only be allowed by `sha256:` of the lower-cased matched text (so the allowlist never
  contains a private value), scoped by the path glob. Prefer rewording or adding the ordinary word to the
  private stop-word list instead.
- Value `*` allows a whole category, but only in the files of a path glob other than `*` and never in commit messages:
  meant for vendored third-party data (e.g. the CLDR emoji names, where name tokens are ordinary words).
- `image` entries pin the reviewed blob (`git hash-object <file>`); without a hash any content passes, so
  always give one.
- The allowlist file itself is checked against the deny-lists only.
