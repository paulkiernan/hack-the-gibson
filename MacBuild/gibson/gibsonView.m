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
    if ( mDrawBackground ) {
        //   draw background after view is installed in a window for the first time
        [[NSColor colorWithDeviceRed: 0.0 green: 0.0
                                blue: 0.0 alpha: 1.0] set];
        [NSBezierPath fillRect: [self bounds]];
        mDrawBackground = NO;
    }
    
    NSRect viewBounds = [self bounds];
    float startingX = SSRandomFloatBetween(
                                           NSMinX( viewBounds ), NSMaxX( viewBounds ) );
    float startingY = SSRandomFloatBetween(
                                           NSMinY( viewBounds ), NSMaxY( viewBounds ) );
    float width = SSRandomFloatBetween(
                                       NSWidth(viewBounds)/20, NSWidth(viewBounds)/2);
    float height = SSRandomFloatBetween(
                                        NSHeight(viewBounds)/20, NSHeight(viewBounds)/2);
    NSRect rectToFill = NSMakeRect( startingX, startingY,
                                   width, height );
    
    float red = SSRandomFloatBetween(0.0, 1.0);
    float green = SSRandomFloatBetween(0.0, 1.0);
    float blue = SSRandomFloatBetween(0.0, 1.0);
    float alpha = SSRandomFloatBetween(0.0, 1.0);
    [[NSColor colorWithDeviceRed: red green: green
                            blue: blue alpha: alpha] set];
    [NSBezierPath fillRect: rectToFill];
}

// advances the animation by one frame at the rate set by the
// preceding method (this can be used for drawing instead of drawRect:)
- (void)animateOneFrame
{
    //   request that our view be redrawn (causes Cocoa to call drawRect:)
    [self setNeedsDisplay: YES];
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

- (void)viewDidMoveToWindow {
    // this NSView method is called when our screen saver view is added to its window
    // we'll use this signal to tell drawRect: to erase the background
    mDrawBackground = YES;
}

- (BOOL)isOpaque {
    // this keeps Cocoa from unneccessarily redrawing our superview
    return YES;
}

@end
