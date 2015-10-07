//
//  gibsonView.h
//  gibson
//
//  Created by Paul Kiernan on 9/29/15.
//  Copyright © 2015 Paul Kiernan. All rights reserved.
//

#import <ScreenSaver/ScreenSaver.h>

@interface gibsonView : ScreenSaverView {
    // keep track of whether or not drawRect: should erase the background
    BOOL mDrawBackground;
}
@end
