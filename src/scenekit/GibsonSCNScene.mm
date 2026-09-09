#import "GibsonSCNScene.h"
#import "GibsonSCNCamera.h"

#import <Metal/Metal.h>
#import <MetalKit/MetalKit.h>
#import <ModelIO/ModelIO.h>
#import <SceneKit/ModelIO.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <vector>

#define TOWER_DIST 30.f
#define TOWER_XSIZE 60
#define TOWER_YSIZE 60
#define TOWER_TEXTURE_COUNT 8
#define TEXT_ANIM_WAIT 0.200
#define MAX_PULSE_SPEED 200.f
#define MIN_PULSE_SPEED 120.f
#define MAX_PULSE_HEIGHT 15.f

static float gibson_rand(float low, float high)
{
    return ((float)std::rand() / (float)RAND_MAX) * (high - low) + low;
}

static SCNGeometry *gibson_geometry_from_obj(NSURL *url)
{
    if (!url)
        return nil;
    NSError *err = nil;
    MDLAsset *asset = [[MDLAsset alloc] initWithURL:url
                                   vertexDescriptor:nil
                                    bufferAllocator:nil
                                  preserveTopology:YES
                                              error:&err];
    if (asset.count < 1)
        return nil;
    SCNNode *tmp = [SCNNode nodeWithMDLObject:[asset objectAtIndex:0]];
    if (tmp.geometry)
        return tmp.geometry;
    for (SCNNode *child in tmp.childNodes)
    {
        if (child.geometry)
            return child.geometry;
        for (SCNNode *grand in child.childNodes)
        {
            if (grand.geometry)
                return grand.geometry;
        }
    }
    return nil;
}

static void gibson_metal_log(NSString *msg)
{
    NSLog(@"The Gibson Metal: %@", msg);
    FILE *fp = fopen("/tmp/gibson-saver.log", "a");
    if (fp)
    {
        fprintf(fp, "%s\n", msg.UTF8String);
        fclose(fp);
    }
}

static NSURL *gibson_shader_file_url(NSString *name, NSString *ext)
{
    NSMutableArray<NSURL *> *candidates = [NSMutableArray array];
    NSBundle *classBundle = [NSBundle bundleForClass:[GibsonSCNScene class]];
    NSURL *fromClass = [classBundle URLForResource:name withExtension:ext];
    if (fromClass)
        [candidates addObject:fromClass];
    NSBundle *main = [NSBundle mainBundle];
    NSURL *fromMain = [main URLForResource:name withExtension:ext];
    if (fromMain)
        [candidates addObject:fromMain];

    NSMutableArray<NSString *> *dirs = [NSMutableArray array];
    auto addDir = ^(NSString *dir) {
        if (dir.length)
            [dirs addObject:dir];
    };
    addDir(classBundle.bundlePath);
    addDir(classBundle.resourcePath);
    addDir(main.bundlePath);
    addDir(main.resourcePath);
    addDir([[NSFileManager defaultManager] currentDirectoryPath]);
    addDir([[[NSFileManager defaultManager] currentDirectoryPath]
               stringByAppendingPathComponent:@"src"]);
    addDir([[[[NSFileManager defaultManager] currentDirectoryPath]
                stringByAppendingPathComponent:@"src"]
               stringByAppendingPathComponent:@"scenekit"]);
    NSString *exe = [[NSProcessInfo processInfo] arguments].firstObject;
    if (exe.length)
    {
        NSString *exeDir = [exe stringByDeletingLastPathComponent];
        addDir(exeDir);
        addDir([exeDir stringByAppendingPathComponent:@"scenekit"]);
    }

    NSString *filename = [name stringByAppendingPathExtension:ext];
    for (NSString *dir in dirs)
        [candidates addObject:[NSURL fileURLWithPath:
            [[dir stringByAppendingPathComponent:filename] stringByStandardizingPath]]];

    NSFileManager *fm = [NSFileManager defaultManager];
    for (NSURL *url in candidates)
    {
        if (url.isFileURL && [fm fileExistsAtPath:url.path])
            return url;
    }
    return nil;
}

static id<MTLLibrary> gibson_shader_library(void)
{
    static id<MTLLibrary> library;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        if (!device)
        {
            gibson_metal_log(@"no Metal device");
            return;
        }
        NSError *err = nil;
        NSURL *libURL = gibson_shader_file_url(@"Gibson", @"metallib");
        if (libURL)
        {
            library = [device newLibraryWithURL:libURL error:&err];
            if (library)
            {
                gibson_metal_log([NSString stringWithFormat:@"loaded %@", libURL.path]);
                return;
            }
            gibson_metal_log([NSString stringWithFormat:@"metallib load failed: %@", err]);
            err = nil;
        }
        NSURL *srcURL = gibson_shader_file_url(@"GibsonShaders", @"metal");
        if (!srcURL)
        {
            gibson_metal_log(@"GibsonShaders.metal not found");
            return;
        }
        NSString *source = [NSString stringWithContentsOfURL:srcURL
                                                    encoding:NSUTF8StringEncoding
                                                       error:&err];
        if (!source)
        {
            gibson_metal_log([NSString stringWithFormat:@"could not read %@: %@", srcURL.path, err]);
            return;
        }
        MTLCompileOptions *opts = [[MTLCompileOptions alloc] init];
        library = [device newLibraryWithSource:source options:opts error:&err];
        if (!library)
            gibson_metal_log([NSString stringWithFormat:@"Metal compile failed: %@", err]);
        else
            gibson_metal_log([NSString stringWithFormat:@"compiled %@", srcURL.path]);
    });
    return library;
}

static void gibson_bind_material(SCNNode *node, SCNMaterial *mat)
{
    if (node.geometry)
        node.geometry.materials = @[ mat ];
    for (SCNNode *child in node.childNodes)
        gibson_bind_material(child, mat);
}

static void gibson_apply_shader_program(SCNMaterial *mat,
                                        NSString *fragment,
                                        BOOL opaque,
                                        id<SCNProgramDelegate> delegate)
{
    id<MTLLibrary> library = gibson_shader_library();
    if (!library)
        return;

    SCNProgram *program = [SCNProgram program];
    program.library = library;
    program.vertexFunctionName = @"gibsonFloorVertex";
    program.fragmentFunctionName = fragment;
    program.opaque = opaque;
    program.delegate = delegate;
    mat.program = program;
    [mat setValue:mat.diffuse forKey:@"diffuseTexture"];
}

static void gibson_apply_floor_program(SCNMaterial *mat, id<SCNProgramDelegate> delegate)
{
    gibson_apply_shader_program(mat, @"gibsonFloorFragment", YES, delegate);
}

static void gibson_apply_tower_program(SCNMaterial *mat, id<SCNProgramDelegate> delegate)
{
    gibson_apply_shader_program(mat, @"gibsonTowerFragment", NO, delegate);
}

static id gibson_metal_texture_contents(NSImage *image)
{
    if (!image)
        return nil;
    CGImageRef cg = [image CGImageForProposedRect:NULL context:nil hints:nil];
    if (!cg)
        return image;
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    MTKTextureLoader *loader = [[MTKTextureLoader alloc] initWithDevice:device];
    NSError *err = nil;
    id<MTLTexture> tex = [loader newTextureWithCGImage:cg
                                               options:@{
        MTKTextureLoaderOptionSRGB : @NO,
        MTKTextureLoaderOptionGenerateMipmaps : @YES
    }
                                                 error:&err];
    if (!tex)
    {
        gibson_metal_log([NSString stringWithFormat:@"floor texture load failed: %@", err]);
        return (__bridge id)cg;
    }
    return tex;
}

static SCNMaterial *gibson_unlit_material(id contents, BOOL alpha)
{
    SCNMaterial *mat = [SCNMaterial material];
    mat.lightingModelName = SCNLightingModelConstant;
    mat.diffuse.contents = contents;
    mat.diffuse.wrapS = SCNWrapModeRepeat;
    mat.diffuse.wrapT = SCNWrapModeRepeat;
    mat.diffuse.magnificationFilter = SCNFilterModeLinear;
    mat.diffuse.minificationFilter = SCNFilterModeLinear;
    mat.diffuse.mipFilter = SCNFilterModeLinear;
    mat.doubleSided = YES;
    mat.writesToDepthBuffer = YES;
    if (alpha)
    {
        mat.transparent.contents = contents;
        mat.transparencyMode = SCNTransparencyModeAOne;
        mat.blendMode = SCNBlendModeAlpha;
    }
    return mat;
}

@interface GibsonSCNScene () <SCNProgramDelegate>
@end

@implementation GibsonSCNScene
{
    NSURL *_mediaURL;
    GibsonSCNCamera *_flyCam;
    SCNNode *_worldRoot;
    SCNMaterial *_darkTowerMat;
    SCNMaterial *_lightTowerMat;
    NSArray *_darkFrames;
    NSArray *_lightFrames;
    NSInteger _towerFrame;
    NSTimeInterval _lastTowerSwap;
    NSTimeInterval _startTime;
    NSTimeInterval _lastTime;
    BOOL _started;
    std::vector<SCNNode *> _pulses;
    std::vector<float> _pulseSpeed;
    int _gridX;
    int _gridY;
}

static GibsonSCNScene *g_previewWorld;
static GibsonSCNScene *g_fullWorld;

+ (instancetype)sharedWorldForPreview:(BOOL)preview mediaURL:(NSURL *)mediaURL
{
    GibsonSCNScene *__strong *slot = preview ? &g_previewWorld : &g_fullWorld;
    if (*slot)
        return *slot;
    GibsonSCNScene *world = [[GibsonSCNScene alloc] initWithPreview:preview mediaURL:mediaURL];
    *slot = world;
    return world;
}

+ (void)configureView:(SCNView *)view preview:(BOOL)preview mediaURL:(NSURL *)mediaURL
{
    GibsonSCNScene *world = [self sharedWorldForPreview:preview mediaURL:mediaURL];
    view.scene = world.scene;
    view.pointOfView = world.cameraNode;
    view.delegate = world;
    view.playing = YES;
    view.loops = YES;
    view.rendersContinuously = YES;
    view.allowsCameraControl = NO;
    view.autoenablesDefaultLighting = NO;
    view.backgroundColor = [NSColor colorWithCalibratedRed:0.0
                                                     green:10.0 / 255.0
                                                      blue:15.0 / 255.0
                                                     alpha:1.0];
    view.antialiasingMode = SCNAntialiasingModeNone;
    view.preferredFramesPerSecond = preview ? 30 : 60;
    view.jitteringEnabled = NO;
    if (@available(macOS 10.15, *))
        view.temporalAntialiasingEnabled = NO;
}

- (SCNNode *)cameraNode
{
    return _flyCam.cameraNode;
}

- (instancetype)initWithPreview:(BOOL)preview mediaURL:(NSURL *)mediaURL
{
    self = [super init];
    if (!self)
        return nil;

    _preview = preview;
    _mediaURL = mediaURL;
    /* Always build the full 60x60 city. A 10x10 preview grid is smaller than
       the flythrough, so the towers look like a finite clump. */
    _gridX = TOWER_XSIZE;
    _gridY = TOWER_YSIZE;
    _towerFrame = 0;
    _started = NO;
    std::srand((unsigned)std::time(nullptr));

    _scene = [SCNScene scene];
    _scene.background.contents = [NSColor colorWithCalibratedRed:0.0
                                                           green:10.0 / 255.0
                                                            blue:15.0 / 255.0
                                                           alpha:1.0];
    /* City edge is ~900 units out. Fade to the clear color before that so the
       grid never shows a hard wall of towers. */
    _scene.fogStartDistance = 480;
    _scene.fogEndDistance = 820;
    _scene.fogDensityExponent = 2.0;
    _scene.fogColor = [NSColor colorWithCalibratedRed:0.0
                                               green:10.0 / 255.0
                                                blue:15.0 / 255.0
                                               alpha:1.0];
    _scene.fogDensityExponent = 1.0;

    /* Irrlicht is left-handed; SceneKit is right-handed. Flip Z so authored
       waypoints and placements match the windowed app. */
    _worldRoot = [SCNNode node];
    _worldRoot.name = @"gibson-world";
    _worldRoot.scale = SCNVector3Make(1, 1, -1);
    [_scene.rootNode addChildNode:_worldRoot];

    [self addLights];
    [self addRoom];
    [self addTowers];
    [self addPulses];

    _flyCam = [[GibsonSCNCamera alloc] initWithRoot:_worldRoot];
    return self;
}

- (NSURL *)mediaItem:(NSString *)name
{
    return [_mediaURL URLByAppendingPathComponent:name];
}

- (NSImage *)imageNamed:(NSString *)name
{
    return [[NSImage alloc] initWithContentsOfURL:[self mediaItem:name]];
}

- (void)addLights
{
    SCNLight *ambient = [SCNLight light];
    ambient.type = SCNLightTypeAmbient;
    ambient.color = [NSColor colorWithCalibratedWhite:0.4 alpha:1.0];
    SCNNode *ambientNode = [SCNNode node];
    ambientNode.light = ambient;
    [_worldRoot addChildNode:ambientNode];

    SCNLight *dir = [SCNLight light];
    dir.type = SCNLightTypeDirectional;
    dir.color = [NSColor colorWithCalibratedWhite:0.8 alpha:1.0];
    SCNNode *dirNode = [SCNNode node];
    dirNode.light = dir;
    dirNode.eulerAngles = SCNVector3Make(-45.f * (float)M_PI / 180.f,
                                         45.f * (float)M_PI / 180.f,
                                         0);
    [_worldRoot addChildNode:dirNode];
}

- (void)addRoom
{
    SCNGeometry *geom = gibson_geometry_from_obj([self mediaItem:@"room.obj"]);
    if (!geom)
        return;
    NSImage *tex = [self imageNamed:@"room.png"];
    SCNMaterial *mat = gibson_unlit_material(gibson_metal_texture_contents(tex), NO);
    gibson_apply_floor_program(mat, self);
    geom.materials = @[ mat ];
    SCNNode *room = [SCNNode nodeWithGeometry:geom];
    room.name = @"room";
    room.eulerAngles = SCNVector3Make(0, 0, (float)M_PI);
    [_worldRoot addChildNode:room];
}

- (void)addTowers
{
    NSMutableArray *dark = [NSMutableArray arrayWithCapacity:TOWER_TEXTURE_COUNT];
    NSMutableArray *light = [NSMutableArray arrayWithCapacity:TOWER_TEXTURE_COUNT];
    for (int i = 1; i <= TOWER_TEXTURE_COUNT; ++i)
    {
        NSImage *d = [self imageNamed:[NSString stringWithFormat:@"towers1-%d.png", i]];
        NSImage *l = [self imageNamed:[NSString stringWithFormat:@"towers2-%d.png", i]];
        [dark addObject:gibson_metal_texture_contents(d) ?: d];
        [light addObject:gibson_metal_texture_contents(l) ?: l];
    }
    _darkFrames = dark;
    _lightFrames = light;

    SCNGeometry *geom = gibson_geometry_from_obj([self mediaItem:@"towers.obj"]);
    if (!geom)
        return;

    _darkTowerMat = gibson_unlit_material(dark[0], YES);
    _lightTowerMat = gibson_unlit_material(light[0], YES);
    gibson_apply_tower_program(_darkTowerMat, self);
    gibson_apply_tower_program(_lightTowerMat, self);
    SCNGeometry *darkGeom = [geom copy];
    darkGeom.materials = @[ _darkTowerMat ];
    SCNGeometry *lightGeom = [geom copy];
    lightGeom.materials = @[ _lightTowerMat ];

    SCNNode *darkGroup = [SCNNode node];
    SCNNode *lightGroup = [SCNNode node];
    const int live = _gridX * _gridY;
    float x = -((_gridX / 2) * TOWER_DIST);
    float z = ((_gridY / 2) * TOWER_DIST);

    auto place = ^(SCNGeometry *g, SCNNode *parent, float px, float pz) {
        SCNNode *tower = [SCNNode nodeWithGeometry:g];
        tower.scale = SCNVector3Make(1.3f, 1.1f, 1.3f);
        tower.position = SCNVector3Make(px, 14.f, pz);
        [parent addChildNode:tower];
    };

    place(darkGeom, darkGroup, x + 9.125f, z);

    for (int i = 1; i < live; ++i)
    {
        x += TOWER_DIST;
        if ((i + 1) % _gridX == 0)
        {
            z -= TOWER_DIST;
            x = -((_gridX / 2) * TOWER_DIST);
        }
        const BOOL lightType = ((int)gibson_rand(0.f, 2.f) > 0);
        place(lightType ? lightGeom : darkGeom, lightType ? lightGroup : darkGroup,
              x + TOWER_DIST / 2.f, z);
    }

    SCNNode *darkFlat = [darkGroup flattenedClone];
    SCNNode *lightFlat = [lightGroup flattenedClone];
    gibson_bind_material(darkFlat, _darkTowerMat);
    gibson_bind_material(lightFlat, _lightTowerMat);
    [_worldRoot addChildNode:darkFlat];
    [_worldRoot addChildNode:lightFlat];
}

- (void)addPulses
{
    SCNGeometry *geom = gibson_geometry_from_obj([self mediaItem:@"pulse.obj"]);
    if (!geom)
        return;
    NSImage *tex = [self imageNamed:@"pulse.png"];
    SCNMaterial *mat = gibson_unlit_material(tex, YES);
    geom.materials = @[ mat ];

    const int count = _preview ? 80 : 280;
    _pulses.reserve((size_t)count);
    _pulseSpeed.reserve((size_t)count);
    for (int i = 0; i < count; ++i)
    {
        SCNNode *pulse = [SCNNode nodeWithGeometry:geom];
        pulse.scale = SCNVector3Make(2.f, 2.f, 4.f);
        [self respawnPulse:pulse];
        [_worldRoot addChildNode:pulse];
        _pulses.push_back(pulse);
        _pulseSpeed.push_back(gibson_rand(MIN_PULSE_SPEED, MAX_PULSE_SPEED));
    }
}

- (void)respawnPulse:(SCNNode *)pulse
{
    const float px = TOWER_DIST * (int)gibson_rand(-_gridX, _gridX);
    const float py = gibson_rand(1.f, MAX_PULSE_HEIGHT);
    const float pz = TOWER_DIST * (int)gibson_rand(-_gridY, _gridY) + TOWER_DIST / 2.f;
    pulse.position = SCNVector3Make(
        px + gibson_rand(-TOWER_DIST / 4.f, TOWER_DIST / 4.f),
        py,
        pz + gibson_rand(-TOWER_DIST / 4.f, TOWER_DIST / 4.f));
    const int yawSteps = (int)gibson_rand(0.f, 4.f);
    pulse.eulerAngles = SCNVector3Make(0, yawSteps * (float)M_PI / 2.f, 0);
}

- (void)renderer:(id<SCNSceneRenderer>)renderer updateAtTime:(NSTimeInterval)time
{
    (void)renderer;
    [self tickAtTime:time];
}

- (void)program:(SCNProgram *)program handleError:(NSError *)error
{
    (void)program;
    gibson_metal_log([NSString stringWithFormat:@"SCNProgram error: %@", error]);
}

- (void)resetClock
{
    _started = NO;
}

- (void)tickAtTime:(NSTimeInterval)time
{
    if (!_started)
    {
        _started = YES;
        _startTime = time;
        _lastTime = time;
        _lastTowerSwap = time;
    }

    if (time < _lastTime)
        return;

    const float delta = (float)std::min(0.1, std::max(0.0, (double)(time - _lastTime)));
    _lastTime = time;

    [_flyCam updateAtTime:time startTime:_startTime];
    [self updateTowersAtTime:time];
    [self updatePulses:delta];
}

- (void)updateTowersAtTime:(NSTimeInterval)time
{
    if (time - _lastTowerSwap < TEXT_ANIM_WAIT)
        return;
    _lastTowerSwap = time;
    _towerFrame = (_towerFrame + 1) % TOWER_TEXTURE_COUNT;
    if (_darkFrames.count)
    {
        id frame = _darkFrames[_towerFrame];
        _darkTowerMat.diffuse.contents = frame;
        _darkTowerMat.transparent.contents = frame;
    }
    if (_lightFrames.count)
    {
        id frame = _lightFrames[_towerFrame];
        _lightTowerMat.diffuse.contents = frame;
        _lightTowerMat.transparent.contents = frame;
    }
}

- (void)updatePulses:(float)delta
{
    const float maxX = ((_gridX / 2) * TOWER_DIST);
    const float maxZ = ((_gridY / 2) * TOWER_DIST);
    for (size_t i = 0; i < _pulses.size(); ++i)
    {
        SCNNode *pulse = _pulses[i];
        const float dist = delta * _pulseSpeed[i];
        simd_float3 step = simd_act(pulse.simdOrientation, simd_make_float3(0, 0, -dist));
        pulse.simdPosition += step;
        const SCNVector3 p = pulse.position;
        if (p.x >= maxX || p.z >= maxZ || p.x <= -maxX || p.z <= -maxZ)
        {
            [self respawnPulse:pulse];
            _pulseSpeed[i] = gibson_rand(MIN_PULSE_SPEED, MAX_PULSE_SPEED);
        }
    }
}

@end
