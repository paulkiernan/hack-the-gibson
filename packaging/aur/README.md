# AUR: `hack-the-gibson`

`PKGBUILD` here builds the project from the GitHub tag tarball and installs

| Path | What |
| --- | --- |
| `/usr/bin/hack-the-gibson` | the binary (built as `gibson-app`) |
| `/usr/share/xscreensaver/config/hack-the-gibson.xml` | the xscreensaver descriptor |
| `/usr/share/licenses/hack-the-gibson/LICENSE` | GPL-3.0-or-later |
| `/usr/share/doc/hack-the-gibson/README.md` | project README |
| `/usr/share/doc/hack-the-gibson/xscreensaver-host.md` | how the xscreensaver host works |

## Package name: `hack-the-gibson`, not `-bin`, not `-git`

A package built from tagged sources takes the bare name. `-bin` is only for a
package that repackages a prebuilt binary (the AUR requires the suffix whenever
the sources are available, which they are), and `-git` is for a rolling build
from a branch. So the submission is `hack-the-gibson`.

Checked at the time of writing: the AUR RPC reports zero results for
`hack-the-gibson`, `hack-the-gibson-bin` and `hack-the-gibson-git`, so none of
these names is taken:

```sh
curl -sS 'https://aur.archlinux.org/rpc/v5/info?arg[]=hack-the-gibson' | python3 -m json.tool
```

## One-time setup

1. Register at <https://aur.archlinux.org/register> (the account is separate
   from the Arch Linux BBS/wiki account).
2. Generate a key dedicated to the AUR, or reuse an existing one:

   ```sh
   ssh-keygen -t ed25519 -f ~/.ssh/aur -C 'aur@archlinux.org'
   ```

   Add `~/.ssh/aur.pub` under <https://aur.archlinux.org/account> (My Account →
   SSH Public Key), then tell ssh which key belongs to the AUR:

   ```
   # ~/.ssh/config
   Host aur.archlinux.org
     IdentityFile ~/.ssh/aur
     User aur
   ```

3. Confirm the key works. A successful connection prints a greetings banner and
   hangs up immediately:

   ```sh
   ssh aur@aur.archlinux.org help
   ```

## First submission

```sh
git clone ssh://aur@aur.archlinux.org/hack-the-gibson.git
cd hack-the-gibson
cp /path/to/packaging/aur/PKGBUILD .
updpkgsums                       # fills in the placeholder sha256sums entry
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m 'Initial import: hack-the-gibson 2.0.3-1'
git push
```

The clone works before the package exists: the AUR creates the repository on
first push, and a new package appears on the website once it holds a `PKGBUILD`.

`.SRCINFO` is generated from the `PKGBUILD` and is what the AUR website parses
to show dependencies and sources. Regenerate it and commit it in the same commit
whenever the `PKGBUILD` changes anything that appears in it - `pkgver`, `pkgrel`,
`source`, `sha256sums`, `depends`, `makedepends`, `optdepends`, `pkgdesc`,
`arch`, `license`. Forgetting this is the most common cause of a stale AUR page.

## Updating for a new release

```sh
# in the AUR clone
# edit pkgver= (and reset pkgrel=1)
updpkgsums
makepkg --printsrcinfo > .SRCINFO
git commit -am 'upgpkg: hack-the-gibson 2.0.4-1'
git push
```

`updpkgsums` re-downloads the source and rewrites `sha256sums`. A force-retag
upstream (`git tag -f 2.0.3`) changes the generated GitHub tarball and therefore
invalidates a recorded digest, so `sha256sums` has to be refreshed whenever the
tag is moved, even if the version number did not change.

## Testing before pushing

`makepkg` alone builds against the host system, which hides missing
dependencies. Build the way the AUR expects users and CI to:

```sh
sudo pacman -S --needed devtools namcap

# clean chroot, built from the extra repositories, run from this directory
extra-x86_64-build

# lint the recipe and the result
namcap PKGBUILD
namcap hack-the-gibson-2.0.3-1-x86_64.pkg.tar.zst
```

`extra-x86_64-build` is the devtools wrapper: it sets up and updates a chroot
under `/var/lib/archbuild/extra-x86_64`, installs `makedepends` inside it, builds,
and runs namcap on the package. The output lands in the current directory.
`pkgctl build` is the newer spelling of the same thing.

Read namcap's output rather than skimming it. The entries that matter here:

- anything reported as **missing** for an ELF soname - the `depends` list in the
  `PKGBUILD` was derived by hand, because the binary dlopens nearly every library
  it uses and namcap can only see linked ones;
- anything reported as **unneeded** - an entry that should move to `optdepends`;
- file conflicts with `xscreensaver`, which would mean the load-bearing
  `hack-the-gibson` naming has been broken somewhere.

Then install it in a throwaway VM or container and check the pieces by hand:

```sh
pacman -Ql hack-the-gibson                  # the five paths above, nothing extra
command -v hack-the-gibson                  # /usr/bin/hack-the-gibson
pacman -Qo /usr/share/xscreensaver/config/hack-the-gibson.xml
hack-the-gibson --help
# the repository's own host smoke test, against the installed binary
bash /path/to/repo/platform/linux/smoke-test.sh /usr/bin/hack-the-gibson
```

## What users have to do after installing

Installing the package does not turn the screensaver on. The user adds one line
to `~/.xscreensaver`:

```
programs: hack-the-gibson
```

No arguments, and in particular no `-root`. Two reasons, both from how
xscreensaver launches hacks:

- The daemon runs the `programs:` line verbatim and hands the window over in
  `$XSCREENSAVER_WINDOW`; it never appends a window-id argument.
  `xscreensaver-settings` appends `--window-id 0x<id>` itself when it builds the
  command for the embedded preview. The binary accepts either.
- `-root` exists in upstream xscreensaver hacks because they implement the
  xscreensaver protocol's root-window mode. This binary has no `-root` switch -
  it adopts the window it is given - so a `programs:` line carrying `-root` would
  fail to start (clap rejects an unknown argument), and the descriptor
  deliberately declares no `<command>` element for the same reason: whatever is
  in there is written into the `programs:` line and would break one of the two
  launch paths.

`xscreensaver` is an `optdepends`, not a dependency, because the same binary is a
standalone desktop app and xscreensaver is X11-only. On a Wayland session the
supported route is the fullscreen mode under `swayidle`:

```sh
swayidle -w timeout 600 'hack-the-gibson --fullscreen' resume 'pkill hack-the-gibson'
```

## What is verified and what is not

Verified from this machine (macOS) with no Arch available:

- `bash -n packaging/aur/PKGBUILD` parses.
- The `source` URL, the tag `2.0.3`, and the tarball's top-level directory
  `hack-the-gibson-2.0.3` were checked by downloading the tarball, so
  `cd "$pkgname-$pkgver"` is correct.
- The `depends` set: see the header comment in the `PKGBUILD` for how each entry
  was traced. Package names were checked against the live
  `archlinux.org/packages/.../files/json/` listings.

Not verified, and needing a first run on Arch before the AUR push:

- that the package builds at all (`extra-x86_64-build` has never run);
- that `depends` is complete and minimal (`namcap` output is unread);
- `check()`: it mirrors the Linux job in `.github/workflows/ci.yml`, but that job
  runs on a GitHub runner image, not in a clean chroot, so the test suite has not
  been shown to pass there. `makepkg --nocheck` skips it if it turns out to need
  something a chroot lacks;
- the `.SRCINFO` output, which can only be produced by `makepkg --printsrcinfo`.
