#import "GibsonSaverView.h"
#import "GibsonSCNScene.h"

#import <QuartzCore/QuartzCore.h>
#import <SceneKit/SceneKit.h>

#include "gibson_settings.h"
#include <cstdio>

static void gibson_log(NSString *msg)
{
    NSLog(@"The Gibson screensaver: %@", msg);
    FILE *fp = fopen("/tmp/gibson-saver.log", "a");
    if (fp)
    {
        fprintf(fp, "%s\n", msg.UTF8String);
        fclose(fp);
    }
}

@implementation GibsonSaverView
{
    SCNView *_scnView;
}

+ (BOOL)performGammaFade
{
    return NO;
}

- (NSURL *)mediaURL
{
    NSString *res = [[NSBundle bundleForClass:[self class]] resourcePath];
    return [NSURL fileURLWithPath:[res stringByAppendingPathComponent:@"media"]];
}

- (void)finishInit
{
    [self setAnimationTimeInterval:1.0 / 60.0];
    [self setWantsLayer:YES];
    self.layer.opaque = YES;
    CGColorRef bg = CGColorCreateGenericRGB(0.0, 10.0 / 255.0, 15.0 / 255.0, 1.0);
    self.layer.backgroundColor = bg;
    CGColorRelease(bg);
}

- (instancetype)initWithFrame:(NSRect)frame isPreview:(BOOL)isPreview
{
    self = [super initWithFrame:frame isPreview:isPreview];
    if (self)
    {
        [self finishInit];
        gibson_log([NSString stringWithFormat:@"init %@ %.0fx%.0f proc=%@",
                    isPreview ? @"preview" : @"full",
                    frame.size.width, frame.size.height,
                    [[NSProcessInfo processInfo] processName]]);
    }
    return self;
}

- (instancetype)initWithCoder:(NSCoder *)coder
{
    self = [super initWithCoder:coder];
    if (self)
    {
        [self finishInit];
        gibson_log(@"init coder");
    }
    return self;
}

- (BOOL)isOpaque { return YES; }
- (BOOL)hasConfigureSheet { return NO; }

- (void)attachSceneIfNeeded
{
    if (_scnView || self.isPreview)
        return;

    NSURL *media = [self mediaURL];
    NSString *towers = [[media path] stringByAppendingPathComponent:@"towers.obj"];
    if (![[NSFileManager defaultManager] fileExistsAtPath:towers])
    {
        gibson_log([NSString stringWithFormat:@"missing media at %@", [media path]]);
        return;
    }

    _scnView = [[SCNView alloc] initWithFrame:self.bounds];
    _scnView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    _scnView.wantsLayer = YES;
    _scnView.layer.opaque = YES;
    /* Retina backing is 4x the pixels on this display; 1x is plenty for a saver. */
    _scnView.layer.contentsScale = 1.0;
    [GibsonSCNScene configureView:_scnView preview:NO mediaURL:media];
    _scnView.preferredFramesPerSecond = 60;
    _scnView.antialiasingMode = SCNAntialiasingModeNone;
    _scnView.playing = NO;
    _scnView.rendersContinuously = NO;
    [self addSubview:_scnView];
    gibson_log(@"SCNView attached");
}

- (void)setPlaying:(BOOL)playing
{
    _scnView.playing = playing;
    _scnView.rendersContinuously = playing;
}

- (void)startAnimation
{
    [super startAnimation];
    NSString *res = [[NSBundle bundleForClass:[self class]] resourcePath];
    gibson_settings_reload(res.fileSystemRepresentation);
    gibson_log([NSString stringWithFormat:@"fly_speed=%.2f bank=%.2f from %s",
                gibson_fly_speed(), gibson_bank_strength(),
                gibson_settings_path()[0] ? gibson_settings_path() : "(default)"]);
    [self attachSceneIfNeeded];
    [[GibsonSCNScene sharedWorldForPreview:NO mediaURL:[self mediaURL]] resetClock];
    [self setPlaying:YES];
    gibson_log([NSString stringWithFormat:@"startAnimation %.0fx%.0f win=%.0fx%.0f",
                self.bounds.size.width, self.bounds.size.height,
                self.window.frame.size.width, self.window.frame.size.height]);
}

- (void)stopAnimation
{
    gibson_log(@"stopAnimation");
    [self setPlaying:NO];
    [super stopAnimation];
}

- (void)animateOneFrame
{
    /* SCNView renders itself while playing. */
}

@end
