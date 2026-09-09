// gibson_ffi.h — C ABI of the Hack the Gibson macOS screen saver bridge.
//
// The Rust staticlib `libgibson_ffi.a` implements these functions. This header
// is the Swift bridging header (-import-objc-header) so Swift calls the same
// declarations the Rust side exports.
//
// Threading contract: every function MUST be called from the main thread.
// The handle returned by gibson_create owns the renderer and its reference to
// the NSView/CAMetalLayer; destroy it (gibson_destroy) before the view dies,
// and nil the handle afterwards — the functions tolerate a NULL handle but
// never a dangling one.
//
// gibson_frame return codes:
//   0 success | 1 null handle | 2 render error | 3 panic caught at boundary

#ifndef GIBSON_FFI_H
#define GIBSON_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define GIBSON_FRAME_OK 0
#define GIBSON_FRAME_ERR_NULL_HANDLE 1
#define GIBSON_FRAME_ERR_RENDER 2
#define GIBSON_FRAME_ERR_PANIC 3

// Create a Gibson instance hosted in `ns_view` (a valid NSView*; NULL yields
// NULL). `width`/`height` are the view size in physical pixels, `scale` the
// window's backing scale factor. `settings_json` is a NULL-terminated UTF-8
// JSON object matching gibson_types::Settings (snake_case field names) or NULL
// for defaults. Returns an opaque handle, or NULL on failure.
void *gibson_create(void *ns_view, uint32_t width, uint32_t height,
                    float scale, const char *settings_json);

// Resize the render target. NULL handle is a no-op.
void gibson_resize(void *handle, uint32_t width, uint32_t height, float scale);

// Advance to `time_seconds` (host monotonic seconds, e.g. CACurrentMediaTime)
// and present one frame. Returns 0 on success; see codes above.
int32_t gibson_frame(void *handle, double time_seconds);

// Destroy the instance and free the handle. NULL handle is a no-op. Do not use
// the handle afterwards.
void gibson_destroy(void *handle);

#ifdef __cplusplus
}
#endif

#endif // GIBSON_FFI_H
