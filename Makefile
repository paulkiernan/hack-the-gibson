# Hack the Gibson - convenience targets. Real logic lives in cargo / the platform
# sub-Makefiles; this file only delegates.
#
# `make help` lists every target, and it is also the default target, so a bare
# `make` prints the list instead of starting a build.
#
# The `saver`/`install-saver`/`uninstall-saver` targets delegate to
# platform/macos/Makefile.

# `check` exists next to `test` only for the disk: a workspace test run with
# default debug info costs several gigabytes of target/, and line tables are all
# a backtrace needs. `test` is left exactly as it was for anyone already using
# it. See the "Tests" section of CONTRIBUTING.md.
DEV_DEBUG := line-tables-only

.DEFAULT_GOAL := help
.PHONY: help app snapshot web saver install-saver uninstall-saver check test fmt fmt-check lint clean

help:
	@echo "Hack the Gibson - make targets:"
	@echo
	@echo "  make app              Run the windowed desktop app (Esc or Q quits)"
	@echo "  make snapshot         Render docs/screenshots/lane.png (1920x1080, t=12 s)"
	@echo "  make web              wasm-pack build crates/gibson-web into web/pkg"
	@echo "  make saver            Build platform/macos/build/Gibson.saver"
	@echo "  make install-saver    Build, then copy the saver to ~/Library/Screen Savers"
	@echo "  make uninstall-saver  Remove the installed saver"
	@echo "  make check            The tests to run before a pull request:"
	@echo "                          CARGO_PROFILE_DEV_DEBUG=$(DEV_DEBUG) \\"
	@echo "                            cargo test --workspace --exclude gibson-web"
	@echo "  make test             The same test command without the debuginfo prefix"
	@echo "  make fmt              cargo fmt --all (needs: rustup component add rustfmt)"
	@echo "  make fmt-check        cargo fmt --all --check - what CI runs"
	@echo "  make lint             cargo clippy --workspace --all-targets -- -D warnings"
	@echo "                        (needs: rustup component add clippy; lint gibson-web"
	@echo "                         with --target wasm32-unknown-unknown - see CONTRIBUTING.md)"
	@echo "  make clean            cargo clean"
	@echo
	@echo "Prerequisites, the crate map and the conventions are in CONTRIBUTING.md."

app:
	cargo run --release -p gibson-app

snapshot:
	@mkdir -p docs/screenshots
	cargo run --release -p gibson-app -- --snapshot docs/screenshots/lane.png --size 1920x1080 --time 12

web:
	wasm-pack build crates/gibson-web --target web --release --out-dir ../../web/pkg

saver:
	$(MAKE) -C platform/macos

install-saver:
	$(MAKE) -C platform/macos install

uninstall-saver:
	$(MAKE) -C platform/macos uninstall

# The command contributors should run before opening a pull request. Identical
# to `test` apart from the debuginfo setting; see DEV_DEBUG at the top.
check:
	CARGO_PROFILE_DEV_DEBUG=$(DEV_DEBUG) cargo test --workspace --exclude gibson-web

test:
	cargo test --workspace --exclude gibson-web

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets -- -D warnings

clean:
	cargo clean
