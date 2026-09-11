#!/usr/bin/env bash
#
# Smoke-test the Linux xscreensaver host against a real X server.
#
# This is the only place the X11 host is ever executed. It reproduces the two
# ways xscreensaver launches a hack and requires that each one attaches to the
# window it was given, presents at least one frame, and exits cleanly when the
# window is destroyed:
#
#   daemon    xscreensaver runs the `programs:` line verbatim and passes the
#             window only in $XSCREENSAVER_WINDOW.
#   preview   xscreensaver-settings appends `--window-id 0x<id>` to the command
#             line for its embedded preview (and sets the environment too).
#
# A hack that starts and draws nothing must fail here: the macOS saver shipped
# black once because "the process did not crash" was mistaken for "it drew".
# The verdict comes from the renderer's own presented-frame counter, which is
# logged on exit by the X11 host.
#
# Usage:  smoke-test.sh [path/to/gibson-app]      (default target/release/gibson-app)
#
# Needs:  an X server on $DISPLAY (CI wraps this in `xvfb-run -a -s "-screen 0
#         800x600x24"`; Xvfb's default 8-bit screen has no visual a Vulkan
#         surface can target), a C compiler with libX11 headers, python3, and a
#         Vulkan driver - a software one such as Mesa's lavapipe is enough,
#         because wgpu's native backend set on Linux is Vulkan only.
#
# Environment knobs: SMOKE_RENDER_SECONDS (window lifetime, default 20),
#         SMOKE_SIZE (default 640x480), SMOKE_LAUNCH_TIMEOUT (default 90).
#
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
binary=${1:-$repo_root/target/release/gibson-app}
descriptor=$repo_root/platform/linux/gibson-screensaver.xml
render_seconds=${SMOKE_RENDER_SECONDS:-20}
size=${SMOKE_SIZE:-640x480}
launch_timeout=${SMOKE_LAUNCH_TIMEOUT:-90}

tmp=$(mktemp -d)
window_pid=""
cleanup() {
  if [ -n "$window_pid" ]; then kill "$window_pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT

fail() {
  # ::error:: is rendered as an annotation by GitHub Actions and is harmless
  # (just text) everywhere else.
  echo "::error::$*" >&2
  echo "smoke-test: FAIL: $*" >&2
  exit 1
}

note() { echo "smoke-test: $*"; }

# ---------------------------------------------------------------- preflight

[ -x "$binary" ] || fail "no executable at $binary (build it with: cargo build --release -p gibson-app)"
command -v cc >/dev/null 2>&1 || fail "no C compiler to build the X11 test window with"
command -v python3 >/dev/null 2>&1 || fail "python3 is needed to check the xscreensaver descriptor"

# The descriptor contract that a packager can silently break: xscreensaver's
# settings dialog finds the descriptor by basename of the installed program
# (hack_xml_file()), so the descriptor's own filename and its `name` attribute
# are the name the binary must be installed under; there must be no <command>
# element, because a <command arg="X"/> is written verbatim into the saved
# `programs:` line while the settings dialog also appends the window id itself,
# so any permanent switch here breaks the daemon and/or the preview; and every
# switch the descriptor can emit has to be a flag the binary accepts, with a
# `%` where the value goes (format_switch() substitutes `%` and nothing else,
# so a slider arg without it emits a valueless switch).
flags=$(python3 - "$descriptor" <<'PY' || exit 1
import os
import sys
import xml.etree.ElementTree as ET

path = sys.argv[1]
installed = os.path.basename(path)

try:
    root = ET.parse(path).getroot()
except ET.ParseError as e:
    sys.exit(f"::error::{path} is not well-formed XML: {e}")

if root.tag != "screensaver":
    sys.exit(f"::error::{path}: root element is <{root.tag}>, not <screensaver>")

name = root.get("name")
if name == "gibson" or installed == "gibson.xml":
    sys.exit(
        f"::error::{path} installs the hack as `gibson`, which collides with "
        "the unrelated gibson hack xscreensaver has shipped since 5.44: it "
        "already owns the gibson, gibson.xml and gibson(6) paths"
    )

if name != installed[: -len(".xml")]:
    sys.exit(
        f"::error::{path}: name={name!r} does not match the descriptor's own "
        f"basename {installed!r}; xscreensaver-settings looks the descriptor up "
        "by the installed program's basename, so the binary, this file and "
        "this attribute must all agree"
    )

command = root.find("command")
if command is not None:
    sys.exit(
        "::error::the descriptor has a <command> element "
        f"(arg={command.get('arg')!r}); it is emitted into the saved "
        "`programs:` line and the settings dialog also appends --window-id, "
        "which breaks the daemon and/or the preview"
    )

# Tags whose `arg` carries a user value, and so must place it with `%`.
value_widgets = {"number", "string", "file"}
flags = set()
for element in root.iter():
    if not isinstance(element.tag, str):
        continue
    arg = element.get("arg")
    if arg:
        if element.tag in value_widgets and "%" not in arg:
            sys.exit(
                f"::error::{installed}: <{element.tag} "
                f"id={element.get('id')!r}> has arg={arg!r} with no '%' "
                "placeholder, so the settings dialog would emit a valueless "
                "switch"
            )
        flags.add(arg.split()[0])
    for attr in ("arg-set", "arg-unset"):
        value = element.get(attr)
        if value:
            flags.add(value.split()[0])

print(" ".join(sorted(flags)))
print(
    f"smoke-test: descriptor {installed}: name={name!r}, no <command> element, "
    f"switches {', '.join(sorted(flags)) or '(none)'}",
    file=sys.stderr,
)
PY
)

help_text=$("$binary" --help)
help_flags=$(grep -o -e '--[a-zA-Z0-9-]*' <<<"$help_text" | sort -u | tr '\n' ' ')
for flag in $flags; do
  case " $help_flags " in
    *" $flag "*) ;;
    *) fail "the descriptor passes $flag, which '$binary --help' does not accept" ;;
  esac
done

[ -n "${DISPLAY:-}" ] || fail "DISPLAY is not set; run this under an X server, e.g. xvfb-run -a -s \"-screen 0 800x600x24\" bash $0"
note "display $DISPLAY, binary $binary"

cc -O1 -o "$tmp/x11-test-window" "$repo_root/platform/linux/x11-test-window.c" -lX11 \
  || fail "cannot compile the X11 test window"

# ------------------------------------------------------------------ the runs

# run_case <label> <env|arg>
run_case() {
  local label=$1 mode=$2
  local id_file="$tmp/id-$mode" window_log="$tmp/window-$mode.log"
  local hack_log="$tmp/hack-$mode.log" xid="" status=0 presented=""

  window_pid=""
  "$tmp/x11-test-window" --size "$size" --seconds "$render_seconds" \
    >"$id_file" 2>"$window_log" &
  window_pid=$!

  local _i
  for _i in $(seq 1 100); do
    xid=$(head -n 1 "$id_file" 2>/dev/null || true)
    [ -n "$xid" ] && break
    kill -0 "$window_pid" 2>/dev/null || break
    sleep 0.1
  done
  if [ -z "$xid" ]; then
    cat "$window_log" >&2
    fail "$label: the X11 test window never came up"
  fi
  note "$label: window $xid via $mode (helper pid $window_pid)"
  sed 's/^x11-test-window: /'"$label"': /' "$window_log" >&2

  # The hack is the only thing left running in the foreground: it must notice
  # the window going away on its own. `timeout` is the backstop, so a hang
  # fails the test instead of wedging the job.
  if [ "$mode" = env ]; then
    env XSCREENSAVER_WINDOW="$xid" RUST_LOG=info \
      timeout --signal=KILL "$launch_timeout" "$binary" >"$hack_log" 2>&1 || status=$?
  else
    env RUST_LOG=info \
      timeout --signal=KILL "$launch_timeout" "$binary" --window-id "$xid" \
      >"$hack_log" 2>&1 || status=$?
  fi

  cat "$hack_log"
  grep -m 1 'adapter' "$hack_log" | sed 's/^/'"$label"': /' >&2 || true

  if [ "$status" = 124 ] || [ "$status" = 137 ]; then
    fail "$label: the hack was still running ${launch_timeout}s after it started - the window was destroyed after ${render_seconds}s, so it did not notice"
  fi
  [ "$status" = 0 ] || fail "$label: the hack exited with status $status (see the log above)"

  grep -q "xscreensaver host running on window $xid" "$hack_log" \
    || fail "$label: the hack never reported adopting window $xid"
  grep -Eq "exiting \((window destroyed|window is gone)\)" "$hack_log" \
    || fail "$label: the hack exited without reporting that its window went away"

  presented=$(sed -n 's/.*presented \([0-9][0-9]*\) frames.*/\1/p' "$hack_log" | tail -n 1)
  [ -n "$presented" ] \
    || fail "$label: no frame summary in the log - did the hack reach the render loop?"
  [ "$presented" -gt 0 ] \
    || fail "$label: the hack adopted window $xid but presented 0 frames"

  note "$label: OK - $presented frames presented, exited cleanly"
  wait "$window_pid" 2>/dev/null || true
  window_pid=""
}

run_case "daemon (XSCREENSAVER_WINDOW)" env
run_case "settings preview (--window-id)" arg

note "PASS"
