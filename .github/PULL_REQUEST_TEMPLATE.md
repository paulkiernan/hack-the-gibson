<!--
Short is the point: a paragraph each is plenty, and anything that does not apply
can be deleted. Build and test commands are in CONTRIBUTING.md.
-->

## What changed, and why

<!--
What the change does, and the reason for it. Link an issue ("Fixes #123") if
there is one.
-->

## Where it was built or run

Tick only what you actually exercised. Ticking the boxes you could test honestly
is more useful than ticking all of them - a reviewer needs to know what has
never run, not to see a full column.

- [ ] macOS screen saver (`make saver`, or `make install-saver`)
- [ ] Windows `.scr`
- [ ] Linux xscreensaver hack
- [ ] Desktop app (`cargo run --release -p gibson-app`)
- [ ] Web build (`make web`, served over HTTP)
- [ ] Build or tests only - none of the hosts were run
- [ ] Nothing here needs a host (docs, CI, contributor files)

## Screenshots, for anything visual

Same command and same seed for both images, please:

```sh
cargo run --release -p gibson-app -- \
  --snapshot out.png --size 1920x1080 --time 12 --seed 42
```

**Before**

<!-- drag the before image here -->

**After**

<!-- drag the after image here -->

## Checks

- [ ] `make check` passes: `cargo test --workspace --exclude gibson-web`, with
      `CARGO_PROFILE_DEV_DEBUG=line-tables-only`
- [ ] `make fmt-check` passes, and `make lint` reports no warnings
- [ ] Commits follow the Conventional Commits convention in CONTRIBUTING.md
- [ ] A change to `gibson-types` is flagged as one - it is the frozen contract,
      so `!` after the type or scope, and a `BREAKING CHANGE:` footer
- [ ] Renderer or shader changes respect the WebGL2 envelope described in
      CONTRIBUTING.md
