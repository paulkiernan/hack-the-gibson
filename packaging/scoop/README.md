# Scoop: `hack-the-gibson.json`

A Scoop manifest for the Windows `.scr`, pinned to the release asset and its
published digest:

| Field | Value |
| --- | --- |
| Version | `2.0.3` |
| Asset | `Gibson.scr` (from release `2.0.3`) |
| SHA256 | `52e676be77cb439e96bac246acc22379e3980aa9fbbeb5e1aecadcaba967fa7f` |

The hash is the one GitHub's own `SHA256SUMS` asset carries for the same
release, not a value re-computed from a local download, so it is the digest of
the bytes users actually get. `Gibson.scr` is the release workflow's
`target/release/gibson-app.exe` copied byte for byte (`Copy-Item`), so the asset
is the binary that CI built and uploaded.

`checkver` + `autoupdate` keep the manifest current: `checkver` follows GitHub's
"latest release" for the repository (prereleases are ignored, and this project
tags bare semver with no `v`), and `autoupdate` rebuilds the URL for the new tag
and re-reads the digest out of that release's `SHA256SUMS`:

```json
"hash": {
  "url": "https://github.com/paulkiernan/hack-the-gibson/releases/download/$version/SHA256SUMS",
  "find": "$sha256\\s+Gibson\\.scr"
}
```

To apply it by hand on a Windows box with Scoop installed:

```powershell
cd <bucket repo>
.\bin\checkver.ps1 hack-the-gibson -u
```

## The Windows choreography, and what is honestly unknown

Where a `.scr` has to live to be selectable, as far as this can be established
from documentation without a Windows machine to try it on:

- **The active screen saver is a registry value, and that part is
  Microsoft-documented.** `HKCU\Control Panel\Desktop\SCRNSAVE.EXE` (REG_SZ)
  names the screen saver executable. Microsoft documents setting it from a file
  path with
  `rundll32.exe desk.cpl,InstallScreenSaver <file>`
  (<https://learn.microsoft.com/en-us/windows/win32/devnotes/scrnsave-exe>), and
  the note there is explicit that Windows writes this value when you pick a
  saver in the Display/Personalization UI, and deletes it if you choose
  *(None)*. Group Policy can supersede it.
- **The drop-down list is populated by scanning `%SystemRoot%\System32`.** This
  is *not* stated by Microsoft anywhere I could find; it is what the community
  documents, what the shell's right-click **Install** verb does (it copies the
  file into `System32` before setting the value), and what matches the observed
  behaviour on Windows 10/11. Treat it as very likely rather than certain.
  On 64-bit systems `SysWOW64` is also mentioned in some sources; a 64-bit
  `.scr` belongs in `System32` either way.
- **Consequence, and the reason this manifest is conservative:** a per-user
  Scoop install puts `Gibson.scr` in `%USERPROFILE%\scoop\apps\hack-the-gibson\<version>`,
  which is a directory Windows does not enumerate, so the saver will *not* be
  offered in Settings. What does work immediately without any install step:
  double-clicking the file, or running `Gibson.scr /s` (full screen) and
  `/c` (opens the settings file in the default editor). Both are handled by the
  binary itself; see `crates/gibson-app/src/saver_args.rs`.

What the manifest therefore does and does not do:

- **Does** install the file, verify its digest, and - only for a global install
  (`scoop install -g`, which is already elevated) - copy it into
  `System32`, exactly as the shell's Install verb would, and remove that copy on
  uninstall.
- **Does not** write any registry value. The assignment of a screen saver is
  something the user does in Settings or via the documented `InstallScreenSaver`
  call; silently setting `SCRNSAVE.EXE` from an installer is unverifiable from
  here, and a mistake in that key is a support burden.
- **Does not** shim the `.scr` onto `PATH`. It is a screen saver, not a CLI tool
  people need to call by name.

Everything above about `System32`, `SysWOW64`, the Install verb's copy step and
the `$global` branch has **never been executed**: this repository was prepared on
macOS, which has no Windows, no Scoop and no PowerShell. The manifest's JSON, its
URLs and its hash are checked; its behaviour is not.

## Route 1: personal bucket (works immediately)

Create a public GitHub repository named `scoop-bucket` (the name matters only in
that the bucket is added by URL, but `scoop-bucket` is the convention users
expect), put the manifest in it at the repository root as
`hack-the-gibson.json`, and commit. That is the whole publishing step - Scoop
buckets are read straight out of the repository, with no review and no
signing gate.

Users then:

```powershell
scoop bucket add paulkiernan https://github.com/paulkiernan/scoop-bucket
scoop install paulkiernan/hack-the-gibson
# or, to get the System32 copy so it appears in Screen Saver Settings,
# from an elevated PowerShell:
scoop install -g paulkiernan/hack-the-gibson
```

Tradeoff: users need the extra `scoop bucket add` line, and `scoop search` only
finds it within that bucket. In exchange the manifest is live the moment it is
pushed, and the owner can iterate on it without a review round-trip.

## Route 2: `ScoopInstaller/Extras`

Extras is the general-purpose official bucket. Submission means a pull request
against <https://github.com/ScoopInstaller/Extras> adding
`bucket/hack-the-gibson.json`.

- Requirements: manifest validity (the bucket's CI validates every manifest
  against its JSON schema), a working `checkver`, and the acceptance criteria at
  <https://github.com/ScoopInstaller/Scoop/wiki/Criteria-for-including-apps-in-the-main-bucket>.
  Extras is also autoupdated hourly by the excavator once `checkver` works, so
  the manifest stops needing hand-edits.
- The real obstacle is not the schema: **a PR is expected to be tested on
  Windows**, and this manifest's install and uninstall paths cannot be tested
  here. The practical order is to ship the personal bucket first, let real users
  hit it, and only then open the Extras PR with the confidence that comes from
  those reports.
- Tradeoff: broad reach through the default bucket, in exchange for a review
  cycle, a stricter `installer.script` bar, and the possibility that a reviewer
  asks for the `System32` copying to move into a documented, tested form.

## Updating for a new release

```powershell
# in the bucket clone, on Windows, with the manifest's version set back
.\bin\checkver.ps1 hack-the-gibson -u
# then check that url, hash and the version all moved together and install it
scoop install hack-the-gibson
scoop uninstall hack-the-gibson
```

If `checkver` is unavailable, bump `version`, the two URLs in `autoupdate`, and
`hash`, taking the new `hash` from the `SHA256SUMS` asset on that release rather
than from a local download.

## Verified / not verified

Checked here: the manifest parses (`python3 -m json.tool`); the version, asset
name and SHA256 match GitHub's `SHA256SUMS` for release `2.0.3`; the asset really
is a PE32+ GUI-subsystem x86-64 executable and its imports include `opengl32.dll`
and `dxgi.dll` (checked with `llvm-objdump -p` on the downloaded asset), which is
consistent with it being the wgpu app.

Not checked, and unverifiable without Windows: that `scoop install` and
`scoop uninstall` do what the scripts say; that `checkver -u` resolves the new
version (it should, but it has never run); that a `System32` copy is what makes
the file appear in Screen Saver Settings on the user's Windows build; and that
the `.scr` itself renders on a real GPU - CI builds it but never runs it.
