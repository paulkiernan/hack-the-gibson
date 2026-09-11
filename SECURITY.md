# Security policy

## Scope, honestly

This is a screensaver. It opens no sockets, listens on no port, reads no
network input, and stores no credentials or personal data. There is no server
component and no account system, so there is no authentication or authorisation
to attack. That rules out most of what a security policy usually covers.

What remains is the input the process takes from outside itself. That is the
surface worth reporting against:

| Surface | Where | Why it is the interesting part |
| --- | --- | --- |
| The C ABI boundary | `crates/gibson-ffi`, header at `platform/macos/include/gibson_ffi.h` | The Swift screen saver passes raw `void*` view handles and integers across the FFI. Handles are tracked in a process-wide registry and every entry point is wrapped in `catch_unwind`, so use-after-free or a panic crossing the boundary is a bug worth a report. |
| OS-supplied arguments | `--window-id <xid>` and `$XSCREENSAVER_WINDOW` (Linux), `/s`, `/p <hwnd>`, `/c` (Windows) | These arrive from xscreensaver or the Windows display applet rather than from a person. A malformed or hostile value should be rejected, not trusted. |
| The macOS bundle | `platform/macos/Makefile`, the packaged `Gibson.saver` | What ends up inside the bundle, and how it is signed, decides what the screen-saver process loads. The bundle is ad-hoc signed and **not notarized**; the download instructions in the README cover clearing the quarantine flag. |
| Web build query parameters | `crates/gibson-web` | Every setting can be set from the URL. Values are clamped to legal ranges on load, so a bad one cannot put the renderer out of bounds - but the wasm runs in the page's origin, so anything that escaped that clamping would matter. |
| The settings file | `gibson.toml`, parsed by `gibson-app` | A malformed or hostile file should be rejected or defaulted, never panic. Unknown keys are ignored and every value is clamped; a file that crashes the desktop host is a bug. |

One thing to know about the deployed demo page: it loads a single third-party
script, the Buy Me a Coffee widget from `cdnjs.buymeacoffee.com`, and nothing
else external. The wasm renderer itself makes no network requests at all.

## Not security issues

- A crash, hang, black screen, or wrong-looking render. Those are bugs; the
  issue templates will get them to the right place faster than an advisory.
- macOS Gatekeeper refusing a downloaded build. The bundle is not notarized by
  design (there is no paid developer account behind this project), and the
  README's `xattr -dr com.apple.quarantine` step is the supported answer.
- Unverified release downloads: verify against `SHA256SUMS` on the release, as
  the README describes.
- A GPU driver or browser that refuses the renderer. Unsupported or blocklisted
  graphics stacks are out of scope.

## Supported versions

Only the tip of `main` and the most recent published release are supported.
Fixes land on `main`; there are no backports to older releases.

## Reporting

Use GitHub's private vulnerability reporting: **Security** tab, then **Report a
vulnerability**, or go straight to
<https://github.com/paulkiernan/gibson-screensaver/security/advisories/new>.
Please do not open a public issue for a vulnerability before it is fixed.

A useful report includes:

- which host, and the version or commit (`git rev-parse --short HEAD` for a
  source build);
- the OS and GPU;
- exactly what the attacker controls (a URL, a window handle, a config file, a
  bundle, a crafted argument);
- what happens as a result, and the log output if there is any.

This is a spare-time project with no bounty programme. Reports are handled on a
best-effort basis, and you will be credited in the advisory unless you ask not
to be.
