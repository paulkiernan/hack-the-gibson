# Homebrew cask: `hack-the-gibson.rb`

A cask for the owner's **personal tap**, using the `screen_saver` stanza so
`brew install --cask` moves `Gibson.saver` into `~/Library/Screen Savers`.

| Field | Value |
| --- | --- |
| Version | `2.0.3` |
| Asset | `Gibson.saver.zip` from release `2.0.3` |
| SHA256 | `eae196169bbe2c8ef30fb84bafcf51ff0de02b7153b607f4c0f93bf06843a06c` |
| Minimum macOS | `>= :sonoma` (14.0), from the bundle's own `LSMinimumSystemVersion` |

The hash is the one in that release's published `SHA256SUMS`, so it describes
the bytes users download.

## Why a personal tap and not `homebrew/cask`

Two hard requirements stand in the way, either one of which is disqualifying on
its own:

1. **Gatekeeper.** `docs.brew.sh/Acceptable-Casks` requires that "apps,
   installers and other executable artefacts that Gatekeeper can assess must
   pass Homebrew's Gatekeeper checks and must not require System Integrity
   Protection or Gatekeeper to be disabled or bypassed". `Gibson.saver` is
   ad-hoc signed and **not notarized**, so a downloaded copy is quarantined and
   fails assessment; the only way to run it is to clear the quarantine flag,
   which is precisely the bypass that clause rules out. Notarization is
   therefore mandatory for `homebrew/cask`, not a nice-to-have.
2. **Notability.** `docs.brew.sh/Package-Acceptance-Policy#notability` requires
   a new package to "demonstrate public interest beyond its author": at least
   30 forks, 30 watchers or 75 stars normally, and at least **90 forks, 90
   watchers or 225 stars for a self-submission by the repository owner**. The
   canonical repository currently has **17 stars, 3 forks and 1 watcher**, so a
   self-submission would be rejected on the numbers alone.

What would change it: notarize `Gibson.saver` with an Apple Developer ID (which
also removes the `xattr` step users currently need, so the cask would stop
shipping a Gatekeeper workaround), and grow the repository past 225 stars or
90 forks or 90 watchers. Both together, and the cask becomes a normal
`homebrew/cask` candidate.

A personal tap carries none of these constraints: `brew tap` fetches a plain
GitHub repository, and casks in it are installed exactly like official ones. The
caveats below stay, because the quarantine problem is real either way.

## Publishing

The tap repository must be named `homebrew-<tap>`; Homebrew resolves
`brew tap paulkiernan/tap` to `paulkiernan/homebrew-tap`. Create it, then:

```sh
git clone git@github.com:paulkiernan/homebrew-tap.git
cd homebrew-tap
mkdir -p Casks/h
cp /path/to/packaging/homebrew/hack-the-gibson.rb Casks/h/
git add Casks/h/hack-the-gibson.rb
git commit -m 'hack-the-gibson 2.0.3'
git push
```

`Casks/h/hack-the-gibson.rb` mirrors the layout used by `homebrew-cask` itself
(casks live under a directory named for the first letter of the token). A tap
also accepts the file at the repository root, but matching the official layout
keeps a future move to `homebrew/cask` a straight copy.

Users then run:

```sh
brew tap paulkiernan/tap
brew install --cask paulkiernan/tap/hack-the-gibson
```

The full token can also be used in one step, which performs the tap implicitly:

```sh
brew install --cask paulkiernan/tap/hack-the-gibson
```

After a new release, bump `version` and `sha256` together:

```sh
shasum -a 256 Gibson.saver.zip    # or read the value out of the release's SHA256SUMS
```

Do not automate the digest from a local download: the value must be the digest
of the published asset, which `SHA256SUMS` already is.

`brew audit --cask` and `brew style` are worth running for tidiness, but expect
`audit` to complain about things that are only required in `homebrew/cask` -
notably a `url` whose host does not match `homepage`, and the quarantine
caveat. Neither is a correctness problem in a tap.

## Notes on the cask's contents

- `screen_saver "Gibson.saver"` installs the bundle into `~/Library/Screen Savers`.
  The stanza's path is relative to the unpacked archive, which contains exactly
  one top-level item: `Gibson.saver` (verified by listing the published zip).
- `depends_on macos: ">= :sonoma"` matches the bundle's `LSMinimumSystemVersion`
  of 14.0 and the `macos-15` runner the release is built on.
- The bundle's display name is **"The Gibson"** (`CFBundleName` and
  `CFBundleDisplayName` in `platform/macos/Info.plist`), which is the name the
  System Settings list shows, hence `name "The Gibson"` rather than the project
  name.
- `zap` removes the desktop app's config directory
  (`~/Library/Application Support/hack-the-gibson`, the path
  `crates/gibson-app/src/config.rs` uses) and the saver's preferences domain.
  The saver stores its options through `ScreenSaverDefaults` under the bundle
  identifier `org.hackthegibson.TheGibson` (`platform/macos/Sources/Settings.swift`),
  which is the domain the plist path is derived from. Whether that lands in
  `~/Library/Preferences/` or in the `ByHost` subdirectory has **not** been
  checked - if a user reports leftovers, `brew zap` output will show the real
  path, and it can be added to the list.

## Verified / not verified

Checked here: `ruby -c packaging/homebrew/hack-the-gibson.rb` passes; the
version, asset name and SHA256 match the release; `CFBundleName`,
`CFBundleDisplayName`, `CFBundleIdentifier` and `LSMinimumSystemVersion` were
read out of `platform/macos/Info.plist` in this repository; and the requirement
texts quoted above were read from `docs.brew.sh` rather than recalled.

Not checked, because no Homebrew tap was created and no cask was installed:
that `brew install --cask` resolves the URL and digest, that the `screen_saver`
stanza places the bundle where System Settings finds it on a current macOS, and
that the cask passes `brew audit` in any form. Installing from the tap is the
first real test, and it should be done on a machine where the saver can then be
selected in System Settings.
