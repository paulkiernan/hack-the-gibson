//
//  gibsonView.m
//  gibson
//
//  Created by Paul Kiernan on 9/29/15.
//  Copyright © 2015 Paul Kiernan. All rights reserved.
//

#import "gibsonView.h"

@implementation gibsonView

// initializes the view with the given frame rect
- (instancetype)initWithFrame:(NSRect)frame isPreview:(BOOL)isPreview
{
    self = [super initWithFrame:frame isPreview:isPreview];
    if (self) {
        // sets the animation rate in seconds
        [self setAnimationTimeInterval:1/30.0];
    }
    return self;
}

// invoked by the screen saver engine when animation should begin
- (void)startAnimation
{
    [super startAnimation];
}

// invoked by the screen saver engine when animation should stop
- (void)stopAnimation
{
    [super stopAnimation];
}

// draws the view
- (void)drawRect:(NSRect)rect
{
    [super drawRect:rect];
}

// advances the animation by one frame at the rate set by the
// preceding method (this can be used for drawing instead of drawRect:)
- (void)animateOneFrame
{
    return;
}

// returns true if the module has a configure sheet
- (BOOL)hasConfigureSheet
{
    return NO;
}

// returns the module's associated configure sheet
- (NSWindow*)configureSheet
{
    return nil;
}

@end
