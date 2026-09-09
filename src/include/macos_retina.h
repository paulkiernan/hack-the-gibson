#ifndef MACOS_RETINA_H_
#define MACOS_RETINA_H_

#ifdef __cplusplus
extern "C" {
#endif

/* Call before createDevice so new NSOpenGLViews get a 1x framebuffer. */
void gibson_prepare_retina_workaround(void);

/* Match the OpenGL backing store to Irrlicht's point-sized viewport. */
void gibson_fix_retina_framebuffer(unsigned width, unsigned height);

/* Cheap per-frame backing-size sync (no AppKit view walk). */
void gibson_sync_gl_backing(unsigned width, unsigned height);

/* Screen size in points (AppKit), not retina pixels. */
void gibson_get_desktop_size(unsigned *width, unsigned *height);

/* Cover the display with a borderless window; keep keyboard and sRGB. */
void gibson_enter_borderless_fullscreen(void);

void gibson_restore_presentation(void);

void gibson_install_quit_monitor(void);

int gibson_should_quit(void);

void gibson_snapshot_windows(void);
void gibson_cloak_new_windows(void);

void gibson_release_host_window(void *window);

/* Attach Irrlicht's current NSOpenGLContext to an NSView. */
void gibson_bind_gl_to_view(void *nsview, unsigned width, unsigned height);

/* Place a host window over a view (fallback when setView cannot draw). */
void gibson_place_window_over_view(void *nswindow, void *nsview);

void gibson_hide_host_window(void *nswindow);

void gibson_flush_gl(void);

#ifdef __cplusplus
}
#endif

#endif
