#ifndef GIBSON_SCN_CAMERA_H_
#define GIBSON_SCN_CAMERA_H_

#import <SceneKit/SceneKit.h>

@interface GibsonSCNCamera : NSObject

@property (nonatomic, readonly) SCNNode *cameraNode;
@property (nonatomic, readonly) SCNNode *lookAtNode;

- (instancetype)initWithRoot:(SCNNode *)root;
- (void)updateAtTime:(NSTimeInterval)time startTime:(NSTimeInterval)startTime;

@end

#endif
