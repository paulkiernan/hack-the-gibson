# Hack the Gibson — convenience targets. Real logic lives in cargo / the platform
# sub-Makefiles; this file only delegates.
#
# The `saver`/`install-saver`/`uninstall-saver` targets delegate to
# platform/macos/Makefile, which Batch 1 creates. Until then those targets fail
# with "No such file or directory" — expected during the scaffold phase.

.PHONY: app snapshot web saver install-saver uninstall-saver test clean

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

test:
	cargo test --workspace --exclude gibson-web

clean:
	cargo clean
