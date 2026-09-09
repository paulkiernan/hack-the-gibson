#import "GibsonSCNScene.h"
#import "GibsonSCNCamera.h"

#import <ModelIO/ModelIO.h>
#import <SceneKit/ModelIO.h>

#include <algorithm>
#include <cmath>
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

static void gibson_bind_material(SCNNode *node, SCNMaterial *mat)
{
    if (node.geometry)
        node.geometry.materials = @[ mat ];
    for (SCNNode *child in node.childNodes)
        gibson_bind_material(child, mat);
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

@implementation GibsonSCNScene
{
    NSURL *_mediaURL;
    GibsonSCNCamera *_flyCam;
    SCNNode *_worldRoot;
    SCNMaterial *_darkTowerMat;
    SCNMaterial *_lightTowerMat;
    NSArray<NSImage *> *_darkFrames;
    NSArray<NSImage *> *_lightFrames;
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
    SCNMaterial *mat = gibson_unlit_material(tex, NO);
    geom.materials = @[ mat ];
    SCNNode *room = [SCNNode nodeWithGeometry:geom];
    room.name = @"room";
    room.eulerAngles = SCNVector3Make(0, 0, (float)M_PI);
    [_worldRoot addChildNode:room];
}

- (void)addTowers
{
    NSMutableArray<NSImage *> *dark = [NSMutableArray arrayWithCapacity:TOWER_TEXTURE_COUNT];
    NSMutableArray<NSImage *> *light = [NSMutableArray arrayWithCapacity:TOWER_TEXTURE_COUNT];
    for (int i = 1; i <= TOWER_TEXTURE_COUNT; ++i)
    {
        [dark addObject:[self imageNamed:[NSString stringWithFormat:@"towers1-%d.png", i]]];
        [light addObject:[self imageNamed:[NSString stringWithFormat:@"towers2-%d.png", i]]];
    }
    _darkFrames = dark;
    _lightFrames = light;

    SCNGeometry *geom = gibson_geometry_from_obj([self mediaItem:@"towers.obj"]);
    if (!geom)
        return;

    _darkTowerMat = gibson_unlit_material(dark[0], YES);
    _lightTowerMat = gibson_unlit_material(light[0], YES);
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
