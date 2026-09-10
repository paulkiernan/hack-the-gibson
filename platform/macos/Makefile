# Hack the Gibson — macOS screen saver bundle, built with Command Line Tools
# only (no Xcode). Recipe proven by PerfectoWeb/Gibson: per-arch Rust
# staticlib, linked into a Swift dylib with swiftc, lipo'd, wrapped in a
# .saver bundle, ad-hoc codesigned.
#
# Prereqs: rustup target add aarch64-apple-darwin x86_64-apple-darwin (or set
# ARCHS to only the targets you have). All paths that can contain spaces are
# quoted (the install dir, ~/Library/Screen Savers, already does).

BUNDLE      := Gibson
PRODUCT     := $(BUNDLE).saver
ROOT        := $(abspath ../..)
BUILD       := $(abspath build)
TARGET_OS   := 14.0
ARCHS       ?= arm64 x86_64
VERSION     := $(shell sed -n 's/^version = "\([^"]*\)"/\1/p' "$(ROOT)/Cargo.toml" | head -1)

SOURCES     := $(wildcard Sources/*.swift)
HEADER      := include/gibson_ffi.h
FRAMEWORKS  := -framework ScreenSaver -framework AppKit -framework QuartzCore \
               -framework Metal -framework Foundation
SWIFTC      := xcrun swiftc
COMMON      := -swift-version 5 -module-name $(BUNDLE) $(FRAMEWORKS)
RELEASE     := -O -wmo

INSTALL_DIR := $(HOME)/Library/Screen Savers

# Cargo triples per lipo arch.
TRIPLE_arm64  := aarch64-apple-darwin
TRIPLE_x86_64 := x86_64-apple-darwin

# One slice target per arch so lipo can depend on them.
SLICES := $(foreach arch,$(ARCHS),$(BUILD)/$(arch)/$(BUNDLE))
# Same list, each element shell-quoted for the lipo command line.
SLICES_QUOTED := $(foreach s,$(SLICES),"$(s)")
$(foreach arch,$(ARCHS),$(eval $(BUILD)/$(arch)/$(BUNDLE): arch := $(arch)))
$(foreach arch,$(ARCHS),$(eval $(BUILD)/$(arch)/$(BUNDLE): triple := $(TRIPLE_$(arch))))

.PHONY: all install uninstall clean force

all: $(BUILD)/$(PRODUCT)

# The Rust sources are deliberately NOT file prerequisites: make cannot track
# cargo's own incremental state, and a stale `libgibson_ffi.a` silently
# shipped in the bundle is exactly the bug that wastes measurement cycles.
# Every build therefore runs cargo (which decides what to recompile) and
# relinks the Swift host against whatever cargo just produced.
force:

# One slice per architecture: build the Rust staticlib, discover the native
# artifacts rustc needs (frameworks/libobjc …), then compile the Swift host
# against both. The native-libs list is captured to a file so make never has
# to remember it across recipes.
#
# The probe (`cargo rustc -- --print native-static-libs`) only prints when
# rustc actually re-links. To force that without touching any tracked source,
# cargo's fingerprint directory for this crate is removed first (pure build
# cache under target/, which is gitignored): cargo then recompiles just this
# crate — dependencies are untouched — and prints the list. The probe is
# best-effort and never fatal: if it still yields nothing (e.g. an unfamiliar
# rustc output format), the build falls back to the known-good Apple framework
# list for this dependency set with a loud warning. `CARGO_TERM_COLOR=never`
# keeps the note parseable even when the caller exports `CARGO_TERM_COLOR=always`
# (CI does), and any residual ANSI escapes are stripped.
$(BUILD)/%/$(BUNDLE): force $(SOURCES) $(HEADER) Makefile
	@mkdir -p "$(dir $@)"
	@for attempt in 1 2; do \
		rm -rf "$(ROOT)/target/$(triple)/release/.fingerprint/gibson-ffi-"*; \
		( cd "$(ROOT)" && MACOSX_DEPLOYMENT_TARGET=$(TARGET_OS) CARGO_TERM_COLOR=never cargo rustc -p gibson-ffi --release --target $(triple) \
			-- --print native-static-libs 2>&1 \
			| perl -pe 's/\e\[[0-9;]*[a-zA-Z]//g' \
			| sed -n -e 's/^[[:space:]]*framework=/-framework /p' -e 's/^[[:space:]]*lib=/-l/p' \
				-e 's/^note: native-static-libs: //p' \
			| tr '\n' ' ' ) > "$(BUILD)/$(arch)/native-libs.txt"; \
		if [ -s "$(BUILD)/$(arch)/native-libs.txt" ]; then break; fi; \
	done; \
	if [ ! -s "$(BUILD)/$(arch)/native-libs.txt" ]; then \
		echo "warning: native-static-libs probe empty for $(arch); using known-good Apple list"; \
		echo '-lSystem -framework QuartzCore -framework CoreGraphics -liconv -framework Metal -framework Foundation -framework CoreFoundation -lobjc -lc -lm' \
			> "$(BUILD)/$(arch)/native-libs.txt"; \
	fi
	@echo "native static libs ($(arch)): $$(cat "$(BUILD)/$(arch)/native-libs.txt")"
	$(SWIFTC) $(COMMON) $(RELEASE) -parse-as-library -emit-library \
		-target $(arch)-apple-macos$(TARGET_OS) \
		-import-objc-header $(HEADER) \
		$(SOURCES) "$(ROOT)/target/$(triple)/release/libgibson_ffi.a" \
		$$(cat "$(BUILD)/$(arch)/native-libs.txt") \
		-o "$@"

$(BUILD)/$(PRODUCT): $(SLICES) Info.plist thumbnail.png thumbnail@2x.png Makefile
	@rm -rf "$@"
	@mkdir -p "$@/Contents/MacOS" "$@/Contents/Resources"
	lipo -create $(SLICES_QUOTED) -output "$@/Contents/MacOS/$(BUNDLE)"
	sed -e 's/@VERSION@/$(VERSION)/g' Info.plist > "$@/Contents/Info.plist"
	printf 'BNDL????' > "$@/Contents/PkgInfo"
	cp thumbnail.png thumbnail@2x.png "$@/Contents/Resources/"
	codesign --force --sign - --timestamp=none "$@"
	@echo "built $(BUILD)/$(PRODUCT) ($(VERSION), $(ARCHS))"

install: $(BUILD)/$(PRODUCT)
	@mkdir -p "$(INSTALL_DIR)"
	@rm -rf "$(INSTALL_DIR)/$(PRODUCT)"
	cp -R "$(BUILD)/$(PRODUCT)" "$(INSTALL_DIR)/$(PRODUCT)"
	@echo "installed to $(INSTALL_DIR)/$(PRODUCT)"
	@echo "pick it in System Settings > Wallpaper > Screen Saver"
	@-killall legacyScreenSaver 2>/dev/null || true

uninstall:
	@rm -rf "$(INSTALL_DIR)/$(PRODUCT)"
	@echo "removed $(INSTALL_DIR)/$(PRODUCT)"

clean:
	rm -rf "$(BUILD)"
