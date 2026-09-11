# Publishing `gibson-screensaver`

Everything needed to get the project into the distribution channels that are
worth the effort, plus the exact runbook for each. Three directories here hold
files to be submitted; two further channels need no files at all and are listed
at the end.

## The release contract these files pin

Every file here pins release **`2.0.3`**, the current published release
(confirmed with `gh release view`, published 2026-09-10). A release is created
by pushing a bare-semver tag, which runs `.github/workflows/release.yml` and
publishes these assets - the list in that workflow's `RELEASE_ASSETS` is the
single source of truth, and if an asset is ever renamed, every file in this
directory needs a matching edit:

| Asset | Consumed by |
| --- | --- |
| `Gibson.saver.zip` | the Homebrew cask, Screensavers Planet, the awesome list |
| `Gibson.scr` | the Scoop manifest |
| `gibson-screensaver-linux-x86_64.tar.gz` | nothing here - the AUR package builds from source; the tarball is for manual installs |
| `gibson-screensaver-macos-universal.tar.gz` | nothing here |
| `gibson-screensaver-web.zip` | nothing here |
| `SHA256SUMS` | the hash source for the Scoop and Homebrew manifests |

The AUR package deliberately builds from the **tag tarball**
(`.../archive/refs/tags/2.0.3.tar.gz`), not from
`gibson-screensaver-linux-x86_64.tar.gz`. That is what lets it take the bare name
`gibson-screensaver` instead of `gibson-screensaver-bin`: the AUR reserves `-bin` for
packages whose sources are not available, and here they are.

## Channels pursued, and the order to do them in

1. **Screensavers Planet + `awesome-macos-screensavers`** - no file here, no
   account gate, minutes of work. Do these first; see the last section.
2. **Scoop, personal bucket** - [`scoop/`](scoop/). No review, no code-signing
   gate, no admin rights for the default install. The manifest is live as soon
   as a two-file repository is pushed.
3. **Homebrew, personal tap** - [`homebrew/`](homebrew/). Same shape: a tap is
   a GitHub repository, so there is no review, but the cask's url and digest
   need the release to exist first (it does).
4. **AUR, source package** - [`aur/`](aur/). The largest audience for a Linux
   screensaver and the only channel here whose package other people will
   rebuild, which is also why it is last: it is the only one with a real
   verification step that has not been possible on this machine. Build it in a
   clean chroot and read `namcap`'s output before pushing; the runbook is in
   [`aur/README.md`](aur/README.md).

Two further channels are worth doing *after* those, in this order, because both
need something that does not exist yet:

- **`ScoopInstaller/Extras`**: a PR is expected to have been tested on Windows,
  and this manifest's install/uninstall scripts have never run. Ship the
  personal bucket, collect real reports, then submit. Details in
  [`scoop/README.md`](scoop/README.md).
- **`homebrew/cask`**: blocked by two independent requirements - notarization,
  and the notability thresholds for a self-submission (90 forks, 90 watchers or
  225 stars; the repository has 17/3/1). Both are explained in
  [`homebrew/README.md`](homebrew/README.md).

## Channels deliberately not pursued

| Channel | Why not |
| --- | --- |
| **winget** | no `.scr` installer type exists; a winget package needs a wrapped, signed installer or a portable zip with a stable, tested install method, and the catalog's validation requires a Windows run. Scoop gets the same users with a manifest that can be written today. |
| **Flathub** | this is a host-integrated app that renders into a window another process owns (and on X11, into xscreensaver's window) rather than living inside its own container; the sandbox model fights the whole design. Flathub also forbids AI-generated submissions. |
| **MacUpdate** | curated submission with a review queue, and its value is a listing rather than a distribution path - the cask and the two hand submissions reach the same users. |
| **SourceForge** | a mirror that rewraps downloads with its own installer, adds nothing the GitHub release does not already provide, and needs a signed binary to avoid being flagged. |
| **Softpedia** | same shape as SourceForge: a mirror with an editorial queue, and its "100% clean" certification is another signing gate. |

## The two channels that need no packaging files

### Screensavers Planet

Submission form: <https://www.screensaversplanet.com/about/submit>

Send `Gibson.saver.zip` (7.0 MiB, well under their 100 MB limit) and, since the
form invites them, a screenshot. Their stated rules are that the file contains
no adware or viruses and that the submitter has the right to distribute it
(both true: GPL-3.0-or-later, and the project's own source). Say plainly in the
submission note that the saver is ad-hoc signed and not notarized, so a
downloaded copy needs one `xattr -dr com.apple.quarantine` command before it
will draw - their reviewers do check downloads, and it is better they hear it
from us than conclude the saver is broken. A link to the GitHub release as the
canonical download is also worth including so future versions track there.

### `awesome-macos-screensavers`

Pull request against <https://github.com/agarrharr/awesome-macos-screensavers>.
Per its `contributing.md`: one suggestion per PR, additions at the bottom of the
relevant category, and the entry format is a heading, a one-line description
starting with a capital and ending with a full stop, an optional price line, and
a screenshot link. The project belongs under **Sci-Fi**:

```markdown
### The Gibson

> Tower-city flythrough from the 1995 film Hackers, recreated with wgpu.

Free (Open Source)

[![](screenshots/the-gibson.png)](https://github.com/paulkiernan/gibson-screensaver)
```

The screenshot has to be committed to that repository, resized to 1000px wide as
the guide asks. The repository already has a suitable image, so no capture is
needed:

```sh
sips -Z 1000 platform/macos/thumbnail.png --out screenshots/the-gibson.png
```

Include in the PR description the link, why it belongs (a real 3D flythrough,
not another clock, and the only one built on wgpu), and the same quarantine
disclosure as above so the reviewer is not surprised by a Gatekeeper warning.

## What is verified, and what each file still needs

| File | Checked on this machine | Still needs |
| --- | --- | --- |
| `aur/PKGBUILD` | `bash -n` parses; the source URL, tag and tarball top-level directory were verified by downloading it; every `depends` entry was traced to a dlopen site in the pinned crate versions and to that package's file list on archlinux.org | a build in a clean chroot (`extra-x86_64-build`), `namcap`, and `updpkgsums` to replace the placeholder `sha256sums` |
| `aur/README.md` | the SSH/push flow, `.SRCINFO` and chroot commands are quoted from the AUR and devtools documentation; the AUR RPC confirms all three candidate package names are unclaimed | first real push, and a first user's report |
| `scoop/gibson-screensaver.json` | `python3 -m json.tool` parses; version, asset name and SHA256 match the release's `SHA256SUMS`; the asset is a PE32+ GUI x86-64 binary | any execution at all on Windows: `scoop install`, `scoop uninstall`, `checkver -u` |
| `homebrew/gibson-screensaver.rb` | `ruby -c` passes; version, asset name and SHA256 match the release; the zip contains exactly one top-level entry, `Gibson.saver`; `CFBundleName`/`LSMinimumSystemVersion` read from `platform/macos/Info.plist` | a real `brew install --cask` from the tap, on a Mac where the saver can then be selected |
| `packaging/README.md` | the asset list matches `RELEASE_ASSETS` in `.github/workflows/release.yml`; the tag and digests match the published release | nothing |

Nothing in this directory can be built or installed on the machine it was
prepared on: there is no Arch Linux, no pacman, no makepkg, no Windows and no
Homebrew tap here. Each subdirectory's README repeats that split for its own
file, and each one says which specific claims were read out of a primary source
(Apple's plist, Microsoft's `SCRNSAVE.EXE` note, the Arch Wiki, the Scoop wiki,
`docs.brew.sh`) rather than assumed.

## Keeping the files in step with a release

On a new tag, in one pass:

```sh
# AUR
cd <aur clone>
# editor: pkgver=2.0.4, pkgrel=1
updpkgsums
makepkg --printsrcinfo > .SRCINFO

# Scoop (on Windows, in the bucket clone)
.\bin\checkver.ps1 gibson-screensaver -u

# Homebrew cask: bump version and sha256 together, sha256 from the release's SHA256SUMS
```

The tag and the `[workspace.package]` version in `Cargo.toml` are asserted equal
by the release workflow's guard job, and the tag has no `v` prefix, so the version
string in all three manifests is the tag verbatim.
