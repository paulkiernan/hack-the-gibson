#import "GibsonSCNScene.h"

#import <Cocoa/Cocoa.h>
#import <Metal/Metal.h>
#import <SceneKit/SceneKit.h>

#include "gibson_settings.h"
#include <cstdio>
#include <cstring>

static NSURL *gibson_find_media(const char *argv0)
{
    NSMutableArray<NSString *> *dirs = [NSMutableArray array];
    NSString *cwd = [[NSFileManager defaultManager] currentDirectoryPath];
    [dirs addObject:cwd];
    [dirs addObject:[cwd stringByAppendingPathComponent:@".."]];
    if (argv0 && argv0[0])
    {
        NSString *exe = [[NSString stringWithUTF8String:argv0] stringByStandardizingPath];
        NSString *dir = [exe stringByDeletingLastPathComponent];
        [dirs addObject:dir];
        [dirs addObject:[dir stringByAppendingPathComponent:@".."]];
    }

    NSFileManager *fm = [NSFileManager defaultManager];
    for (NSString *dir in dirs)
    {
        NSString *media = [[dir stringByAppendingPathComponent:@"media"] stringByStandardizingPath];
        if ([fm fileExistsAtPath:[media stringByAppendingPathComponent:@"towers.obj"]])
            return [NSURL fileURLWithPath:media];
    }
    return nil;
}

static int gibson_write_snapshot(NSURL *media, const char *outPath)
{
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (!device)
    {
        std::fprintf(stderr, "No Metal device.\n");
        return 1;
    }
    GibsonSCNScene *world = [GibsonSCNScene sharedWorldForPreview:NO mediaURL:media];
    SCNRenderer *renderer = [SCNRenderer rendererWithDevice:device options:nil];
    renderer.scene = world.scene;
    renderer.pointOfView = world.cameraNode;
    renderer.autoenablesDefaultLighting = NO;
    const NSTimeInterval time = CACurrentMediaTime();
    [world tickAtTime:time];
    NSImage *image = [renderer snapshotAtTime:time
                                     withSize:NSMakeSize(1280, 800)
                             antialiasingMode:SCNAntialiasingModeNone];
    if (!image)
    {
        std::fprintf(stderr, "SCNRenderer snapshot failed.\n");
        return 1;
    }
    NSData *tiff = [image TIFFRepresentation];
    NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:tiff];
    NSData *png = [rep representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
    if (![png writeToFile:[NSString stringWithUTF8String:outPath] atomically:YES])
    {
        std::fprintf(stderr, "Could not write %s\n", outPath);
        return 1;
    }
    std::fprintf(stderr, "Wrote %s\n", outPath);
    return 0;
}

int main(int argc, char **argv)
{
    BOOL preview = NO;
    const char *snapshotPath = nullptr;
    for (int i = 1; i < argc; ++i)
    {
        if (std::strcmp(argv[i], "--preview") == 0)
            preview = YES;
        else if (std::strcmp(argv[i], "--snapshot") == 0 && i + 1 < argc)
            snapshotPath = argv[++i];
        else if (std::strcmp(argv[i], "--help") == 0 || std::strcmp(argv[i], "-h") == 0)
        {
            std::fprintf(stderr, "Usage: %s [--preview] [--snapshot out.png]\n", argv[0]);
            return 0;
        }
    }

    @autoreleasepool
    {
        NSURL *media = gibson_find_media(argv[0]);
        if (!media)
        {
            std::fprintf(stderr, "Could not find media/. Run from the project root or src/.\n");
            return 1;
        }
        NSString *root = [[media path] stringByDeletingLastPathComponent];
        gibson_settings_reload(root.fileSystemRepresentation);

        [NSApplication sharedApplication];
        if (snapshotPath)
            return gibson_write_snapshot(media, snapshotPath);

        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

        const NSRect frame = NSMakeRect(0, 0, 1280, 800);
        NSWindow *window = [[NSWindow alloc]
            initWithContentRect:frame
                      styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                 NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable)
                        backing:NSBackingStoreBuffered
                          defer:NO];
        window.title = preview ? @"The Gibson (SceneKit preview)" : @"The Gibson (SceneKit)";
        window.releasedWhenClosed = NO;

        SCNView *view = [[SCNView alloc] initWithFrame:frame];
        view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [GibsonSCNScene configureView:view preview:preview mediaURL:media];
        window.contentView = view;

        [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                              handler:^NSEvent *(NSEvent *event) {
            if (event.keyCode == 53)
            {
                [NSApp terminate:nil];
                return nil;
            }
            NSEventModifierFlags mods =
                event.modifierFlags & NSEventModifierFlagDeviceIndependentFlagsMask;
            NSString *chars = event.charactersIgnoringModifiers.lowercaseString;
            if ((mods & NSEventModifierFlagCommand) &&
                ([chars isEqualToString:@"q"] || [chars isEqualToString:@"w"]))
            {
                [NSApp terminate:nil];
                return nil;
            }
            if ([chars isEqualToString:@"q"])
            {
                [NSApp terminate:nil];
                return nil;
            }
            return event;
        }];

        [[NSNotificationCenter defaultCenter]
            addObserverForName:NSWindowWillCloseNotification
                        object:window
                         queue:nil
                    usingBlock:^(NSNotification *) {
                        [NSApp terminate:nil];
                    }];

        [window center];
        [window makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
        [NSApp run];
    }
    return 0;
}
