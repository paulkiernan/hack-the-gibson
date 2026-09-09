#import <AppKit/AppKit.h>
#import <OpenGL/OpenGL.h>
#import <CoreGraphics/CoreGraphics.h>
#import <objc/runtime.h>

#include "macos_retina.h"

extern bool gibson_is_embedded();

static int g_quit = 0;
static int g_fullscreen = 0;

static BOOL gibson_wants_retina_surface(id, SEL)
{
    return NO;
}

static void gibson_set_wants_retina_surface(id, SEL, BOOL)
{
}

static BOOL gibson_accepts_first_responder(id, SEL)
{
    return YES;
}

static void gibson_disable_retina_on_view(NSView *view)
{
    if ([view respondsToSelector:@selector(setWantsBestResolutionOpenGLSurface:)])
        [(id)view setWantsBestResolutionOpenGLSurface:NO];

    for (NSView *child in view.subviews)
        gibson_disable_retina_on_view(child);
}

static void gibson_apply_srgb(NSWindow *window)
{
    NSColorSpace *srgb = [NSColorSpace sRGBColorSpace];
    if ([window respondsToSelector:@selector(setColorSpace:)])
        [window setColorSpace:srgb];

    NSOpenGLContext *gl = [NSOpenGLContext currentContext];
    if (gl && [gl respondsToSelector:@selector(setColorSpace:)])
        [(id)gl setColorSpace:srgb];
}

static NSEvent *gibson_handle_key(NSEvent *event)
{
    if (event.type != NSEventTypeKeyDown)
        return event;

    if (event.keyCode == 53)
    {
        g_quit = 1;
        return nil;
    }

    NSEventModifierFlags mods = event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
    NSString *chars = event.charactersIgnoringModifiers.lowercaseString;
    const BOOL command = (mods & NSEventModifierFlagCommand) != 0;

    if (command && ([chars isEqualToString:@"q"] || [chars isEqualToString:@"."]))
    {
        g_quit = 1;
        return nil;
    }

    return event;
}

void gibson_prepare_retina_workaround(void)
{
    [NSApplication sharedApplication];
    if (!gibson_is_embedded())
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    /* Do not swizzle NSOpenGLView inside Screen Saver Engine / System Settings. */
    if (gibson_is_embedded())
        return;

    Class cls = NSClassFromString(@"NSOpenGLView");
    if (!cls)
        return;

    Method getMethod = class_getInstanceMethod(cls, @selector(wantsBestResolutionOpenGLSurface));
    if (getMethod)
        method_setImplementation(getMethod, (IMP)gibson_wants_retina_surface);

    Method setMethod = class_getInstanceMethod(cls, @selector(setWantsBestResolutionOpenGLSurface:));
    if (setMethod)
        method_setImplementation(setMethod, (IMP)gibson_set_wants_retina_surface);

    Method first = class_getInstanceMethod(cls, @selector(acceptsFirstResponder));
    if (first)
        method_setImplementation(first, (IMP)gibson_accepts_first_responder);
}

void gibson_sync_gl_backing(unsigned width, unsigned height)
{
    if (width == 0 || height == 0)
        return;

    CGLContextObj ctx = CGLGetCurrentContext();
    if (!ctx)
        return;

    GLint dim[2] = { (GLint)width, (GLint)height };
    CGLSetParameter(ctx, kCGLCPSurfaceBackingSize, dim);
    CGLEnable(ctx, kCGLCESurfaceBackingSize);
}

void gibson_fix_retina_framebuffer(unsigned width, unsigned height)
{
    NSApplication *app = [NSApplication sharedApplication];
    for (NSWindow *window in app.windows)
    {
        if (window.contentView)
            gibson_disable_retina_on_view(window.contentView);
        gibson_apply_srgb(window);
        [window makeFirstResponder:window.contentView];
    }

    gibson_sync_gl_backing(width, height);
}

void gibson_get_desktop_size(unsigned *width, unsigned *height)
{
    NSRect frame = [[NSScreen mainScreen] frame];
    if (width)
        *width = (unsigned)frame.size.width;
    if (height)
        *height = (unsigned)frame.size.height;
}

void gibson_enter_borderless_fullscreen(void)
{
    g_fullscreen = 1;

    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    [NSApp activateIgnoringOtherApps:YES];
    [NSApp setPresentationOptions:(NSApplicationPresentationHideDock |
                                   NSApplicationPresentationHideMenuBar |
                                   NSApplicationPresentationDisableAppleMenu)];

    NSScreen *screen = [NSScreen mainScreen];
    NSRect frame = screen.frame;

    for (NSWindow *window in [NSApp windows])
    {
        [window setStyleMask:NSWindowStyleMaskBorderless];
        [window setFrame:frame display:YES];
        [window setLevel:NSMainMenuWindowLevel + 1];
        [window setHidesOnDeactivate:NO];
        [window setOpaque:YES];
        [window setHasShadow:NO];
        [window setCollectionBehavior:NSWindowCollectionBehaviorStationary |
                                      NSWindowCollectionBehaviorCanJoinAllSpaces |
                                      NSWindowCollectionBehaviorFullScreenAuxiliary];
        gibson_apply_srgb(window);
        [window makeKeyAndOrderFront:nil];
        [window makeFirstResponder:window.contentView];
        if (window.contentView)
            gibson_disable_retina_on_view(window.contentView);
    }
}

void gibson_restore_presentation(void)
{
    if (!g_fullscreen)
        return;

    [NSApp setPresentationOptions:NSApplicationPresentationDefault];
    [NSMenu setMenuBarVisible:YES];
    g_fullscreen = 0;
}

void gibson_install_quit_monitor(void)
{
    [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                          handler:^NSEvent *(NSEvent *event) {
                                              return gibson_handle_key(event);
                                          }];
}

int gibson_should_quit(void)
{
    return g_quit;
}

static NSArray *g_windows_before = nil;

void gibson_snapshot_windows(void)
{
    [g_windows_before release];
    g_windows_before = [[[NSApp windows] copy] retain];
}

void gibson_cloak_new_windows(void)
{
    for (NSWindow *window in [NSApp windows])
    {
        if (g_windows_before && [g_windows_before containsObject:window])
            continue;

        [window setAlphaValue:0.0];
        [window setIgnoresMouseEvents:YES];
        [window setHasShadow:NO];
        [window setHidesOnDeactivate:NO];
        [window setExcludedFromWindowsMenu:YES];
        [window setCollectionBehavior:NSWindowCollectionBehaviorTransient |
                                      NSWindowCollectionBehaviorIgnoresCycle];
        [window setLevel:kCGMinimumWindowLevel];
        [window orderBack:nil];
        if (window.contentView)
            gibson_disable_retina_on_view(window.contentView);
        gibson_apply_srgb(window);
    }
}

void *gibson_create_host_window(unsigned width, unsigned height)
{
    if (width < 64)
        width = 64;
    if (height < 64)
        height = 64;

    /* Keep the window on a real display so CGL gives us a usable context
     * for render-to-texture. Alpha 0 keeps it invisible. */
    NSScreen *screen = [NSScreen mainScreen];
    NSRect vf = screen ? screen.visibleFrame : NSMakeRect(0, 0, 1280, 800);
    NSRect frame = NSMakeRect(NSMinX(vf), NSMinY(vf) - 8, 256, 256);
    NSWindow *window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:NSWindowStyleMaskBorderless
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    [window setReleasedWhenClosed:NO];
    [window setOpaque:NO];
    [window setHasShadow:NO];
    [window setBackgroundColor:[NSColor clearColor]];
    [window setAlphaValue:0.0];
    [window setIgnoresMouseEvents:YES];
    [window setHidesOnDeactivate:NO];
    [window setLevel:kCGMinimumWindowLevel];
    [window setCollectionBehavior:NSWindowCollectionBehaviorTransient |
                                  NSWindowCollectionBehaviorIgnoresCycle];
    [window setExcludedFromWindowsMenu:YES];
    gibson_apply_srgb(window);
    if (window.contentView)
        gibson_disable_retina_on_view(window.contentView);
    [window orderBack:nil];
    (void)width;
    (void)height;
    return (void *)window;
}

void gibson_release_host_window(void *window)
{
    if (!window)
        return;
    NSWindow *host = (NSWindow *)window;
    NSWindow *parent = host.parentWindow;
    if (parent)
        [parent removeChildWindow:host];
    [host orderOut:nil];
    [host setContentView:nil];
    [host release];
}

void gibson_bind_gl_to_view(void *nsview, unsigned width, unsigned height)
{
    if (!nsview)
        return;

    NSView *view = (NSView *)nsview;
    if ([view respondsToSelector:@selector(setWantsLayer:)])
        [view setWantsLayer:NO];
    gibson_disable_retina_on_view(view);

    NSOpenGLContext *gl = [NSOpenGLContext currentContext];
    if (!gl)
        return;

    [gl setView:view];
    [gl update];
    [gl makeCurrentContext];
    gibson_apply_srgb(view.window);
    gibson_sync_gl_backing(width, height);
}

void gibson_place_window_over_view(void *nswindow, void *nsview)
{
    if (!nswindow || !nsview)
        return;

    NSWindow *host = (NSWindow *)nswindow;
    NSView *view = (NSView *)nsview;
    NSWindow *parent = view.window;
    if (!parent)
        return;

    NSRect local = view.bounds;
    NSRect inWindow = [view convertRect:local toView:nil];
    NSRect screen = [parent convertRectToScreen:inWindow];

    if (host.parentWindow && host.parentWindow != parent)
        [host.parentWindow removeChildWindow:host];

    [host setStyleMask:NSWindowStyleMaskBorderless];
    [host setFrame:screen display:YES];
    [host setIgnoresMouseEvents:YES];
    [host setHidesOnDeactivate:NO];
    [host setLevel:parent.level];
    gibson_apply_srgb(host);
    if (host.contentView)
        gibson_disable_retina_on_view(host.contentView);

    if (host.parentWindow != parent)
        [parent addChildWindow:host ordered:NSWindowAbove];
    [host orderFront:nil];
    [parent makeKeyWindow];
}

void gibson_hide_host_window(void *nswindow)
{
    if (!nswindow)
        return;
    NSWindow *host = (NSWindow *)nswindow;
    NSWindow *parent = host.parentWindow;
    if (parent)
        [parent removeChildWindow:host];
    [host orderOut:nil];
}

void gibson_flush_gl(void)
{
    NSOpenGLContext *gl = [NSOpenGLContext currentContext];
    if (gl)
        [gl flushBuffer];
}
