#ifndef GIBSON_SCN_SCENE_H_
#define GIBSON_SCN_SCENE_H_

#import <SceneKit/SceneKit.h>

@interface GibsonSCNScene : NSObject <SCNSceneRendererDelegate>

@property (nonatomic, readonly) SCNScene *scene;
@property (nonatomic, readonly) SCNNode *cameraNode;
@property (nonatomic, readonly) BOOL preview;

+ (instancetype)sharedWorldForPreview:(BOOL)preview mediaURL:(NSURL *)mediaURL;
+ (void)configureView:(SCNView *)view preview:(BOOL)preview mediaURL:(NSURL *)mediaURL;
- (void)tickAtTime:(NSTimeInterval)time;
- (void)resetClock;

@end

#endif
