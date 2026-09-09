#import "GibsonSCNCamera.h"

#include "fly_path.h"
#include "gibson_settings.h"

#include <algorithm>
#include <cmath>

#import <simd/simd.h>

@implementation GibsonSCNCamera
{
    float _bank;
    float _yawRate;
    float _lastElapsedMs;
}

static int gibson_clamp_wrap(int idx, int size)
{
    if (idx < 0)
        return size + idx;
    if (idx >= size)
        return idx - size;
    return idx;
}

static SCNVector3 gibson_spline_at(float dt)
{
    const int pSize = GIBSON_FLY_PATH_COUNT;
    const int unwrappedIdx = (int)std::floor(dt);
    const int span = pSize - 1;
    const bool pingPong = true;
    const bool pong = pingPong && span > 0 && ((unwrappedIdx / span) % 2) != 0;
    float u = dt - std::floor(dt);
    if (pong)
        u = 1.f - u;

    int idx;
    if (pong)
        idx = (span - 1) - (unwrappedIdx % span);
    else if (pingPong)
        idx = unwrappedIdx % span;
    else
        idx = unwrappedIdx % pSize;

    const int i0 = gibson_clamp_wrap(idx - 1, pSize);
    const int i1 = gibson_clamp_wrap(idx + 0, pSize);
    const int i2 = gibson_clamp_wrap(idx + 1, pSize);
    const int i3 = gibson_clamp_wrap(idx + 2, pSize);

    const float h1 = 2.f * u * u * u - 3.f * u * u + 1.f;
    const float h2 = -2.f * u * u * u + 3.f * u * u;
    const float h3 = u * u * u - 2.f * u * u + u;
    const float h4 = u * u * u - u * u;
    const float tightness = GIBSON_FLY_TIGHTNESS;

    auto pt = [](int i) {
        return SCNVector3Make(GIBSON_FLY_PATH[i][0], GIBSON_FLY_PATH[i][1], GIBSON_FLY_PATH[i][2]);
    };

    const SCNVector3 p0 = pt(i0);
    const SCNVector3 p1 = pt(i1);
    const SCNVector3 p2 = pt(i2);
    const SCNVector3 p3 = pt(i3);

    const SCNVector3 t1 = SCNVector3Make(
        (p2.x - p0.x) * tightness,
        (p2.y - p0.y) * tightness,
        (p2.z - p0.z) * tightness);
    const SCNVector3 t2 = SCNVector3Make(
        (p3.x - p1.x) * tightness,
        (p3.y - p1.y) * tightness,
        (p3.z - p1.z) * tightness);

    return SCNVector3Make(
        p1.x * h1 + p2.x * h2 + t1.x * h3 + t2.x * h4,
        p1.y * h1 + p2.y * h2 + t1.y * h3 + t2.y * h4,
        p1.z * h1 + p2.z * h2 + t1.z * h3 + t2.z * h4);
}

static simd_float3 gibson_v(SCNVector3 p)
{
    return simd_make_float3((float)p.x, (float)p.y, (float)p.z);
}

static simd_float3 gibson_safe_norm(simd_float3 v, simd_float3 fallback)
{
    const float len = simd_length(v);
    if (len < 1e-5f)
        return fallback;
    return v / len;
}

- (instancetype)initWithRoot:(SCNNode *)root
{
    self = [super init];
    if (!self)
        return nil;

    _bank = 0.f;
    _yawRate = 0.f;
    _lastElapsedMs = -1.f;

    _lookAtNode = [SCNNode node];
    _lookAtNode.name = @"gibson-lookat";
    [root addChildNode:_lookAtNode];

    SCNCamera *cam = [SCNCamera camera];
    cam.fieldOfView = 72.0;
    cam.zNear = 0.5;
    cam.zFar = 900.0;
    cam.wantsHDR = NO;

    _cameraNode = [SCNNode node];
    _cameraNode.name = @"gibson-camera";
    _cameraNode.camera = cam;
    [root addChildNode:_cameraNode];

    const SCNVector3 start = gibson_spline_at(0);
    _lookAtNode.position = start;
    _cameraNode.position = start;
    return self;
}

- (void)aimWithForward:(simd_float3)forward up:(simd_float3)up position:(simd_float3)pos
{
    simd_float3 z = gibson_safe_norm(-forward, simd_make_float3(0, 0, 1));
    simd_float3 x = simd_cross(up, z);
    if (simd_length(x) < 1e-5f)
        x = simd_cross(simd_make_float3(1, 0, 0), z);
    x = simd_normalize(x);
    simd_float3 y = simd_normalize(simd_cross(z, x));
    const simd_float3x3 rot = simd_matrix(x, y, z);
    _cameraNode.simdPosition = pos;
    _cameraNode.simdOrientation = simd_quaternion(rot);
}

- (void)updateAtTime:(NSTimeInterval)time startTime:(NSTimeInterval)startTime
{
    const float speed = gibson_fly_speed();
    const float elapsedMs = (float)((time - startTime) * 1000.0);
    if (_lastElapsedMs >= 0.f && elapsedMs + 50.f < _lastElapsedMs)
    {
        _bank = 0.f;
        _yawRate = 0.f;
    }
    const float frameDt = _lastElapsedMs < 0.f
        ? (1.f / 60.f)
        : std::max(1.f / 120.f, std::min(0.1f, (elapsedMs - _lastElapsedMs) * 0.001f));
    _lastElapsedMs = elapsedMs;

    const float lookDt = std::max(0.f, elapsedMs) * speed * 0.001f;
    const float camDt = std::max(0.f, elapsedMs - (float)GIBSON_FLY_LOOKAHEAD_MS) * speed * 0.001f;
    const SCNVector3 lookPos = gibson_spline_at(lookDt);
    const SCNVector3 camPos = gibson_spline_at(camDt);
    _lookAtNode.position = lookPos;

    const simd_float3 pos = gibson_v(camPos);
    simd_float3 forward = gibson_v(lookPos) - pos;
    forward = gibson_safe_norm(forward, simd_make_float3(0, 0, 1));

    const float eps = 0.14f;
    const simd_float3 tPrev = gibson_v(gibson_spline_at(camDt - eps)) - gibson_v(gibson_spline_at(camDt - 2.f * eps));
    const simd_float3 tNext = gibson_v(gibson_spline_at(camDt + eps)) - gibson_v(gibson_spline_at(camDt));
    const simd_float3 a = gibson_safe_norm(tPrev, forward);
    const simd_float3 b = gibson_safe_norm(tNext, forward);
    float signedYaw = simd_dot(simd_cross(a, b), simd_make_float3(0, 1, 0));
    if (std::fabs(signedYaw) > 0.45f)
        signedYaw = 0.f;

    const float rawRate = signedYaw / eps;
    const float rateFollow = 1.f - std::exp(-frameDt / 0.28f);
    _yawRate += (rawRate - _yawRate) * rateFollow;

    const float maxBank = gibson_bank_max_degrees() * ((float)M_PI / 180.f);
    float target = -_yawRate * gibson_bank_strength();
    target = std::max(-maxBank, std::min(maxBank, target));

    const float tau = std::max(0.12f, gibson_bank_smoothing());
    const float bankFollow = 1.f - std::exp(-frameDt / tau);
    _bank += (target - _bank) * bankFollow;

    simd_float3 worldUp = simd_make_float3(0, 1, 0);
    if (std::fabs(_bank) > 1e-4f)
    {
        const simd_quatf roll = simd_quaternion(_bank, forward);
        worldUp = simd_normalize(simd_act(roll, worldUp));
        if (simd_dot(worldUp, simd_make_float3(0, 1, 0)) < 0.15f)
            worldUp = simd_make_float3(0, 1, 0);
    }

    [self aimWithForward:forward up:worldUp position:pos];
}

@end
