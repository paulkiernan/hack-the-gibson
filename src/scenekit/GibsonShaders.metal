#include <metal_stdlib>
using namespace metal;

/* Layout matches SceneKit's scn_metal header. Custom programs must take
   scn_frame at buffer(0) and scn_node at buffer(1) or SCNView draws magenta. */
enum {
    SCNVertexSemanticPosition,
    SCNVertexSemanticNormal,
    SCNVertexSemanticTangent,
    SCNVertexSemanticColor,
    SCNVertexSemanticBoneIndices,
    SCNVertexSemanticBoneWeights,
    SCNVertexSemanticTexcoord0
};

struct SCNSceneBuffer {
    float4x4 viewTransform;
    float4x4 inverseViewTransform;
    float4x4 projectionTransform;
    float4x4 viewProjectionTransform;
    float4x4 viewToCubeTransform;
    float4x4 lastFrameViewProjectionTransform;
    float4 ambientLightingColor;
    float4 fogColor;
    float3 fogParameters;
    float2 inverseResolution;
    float time;
    float sinTime;
    float cosTime;
    float random01;
    float motionBlurIntensity;
    float environmentIntensity;
    float4x4 inverseProjectionTransform;
    float4x4 inverseViewProjectionTransform;
    float2 nearFar;
    float4 viewportSize;
    float4x4 inverseTransposeViewTransform;
    float4 clusterScale;
};

struct GibsonNodeBuffer {
    float4x4 modelTransform;
    float4x4 inverseModelTransform;
    float4x4 modelViewTransform;
    float4x4 inverseModelViewTransform;
    float4x4 normalTransform;
    float4x4 modelViewProjectionTransform;
};

struct GibsonFloorVertexIn {
    float3 position [[attribute(SCNVertexSemanticPosition)]];
    float2 texcoord [[attribute(SCNVertexSemanticTexcoord0)]];
};

struct GibsonFloorVertexOut {
    float4 position [[position]];
    float2 texcoord;
    float3 viewPosition;
};

constant float kGradientWidth = 48.0;
constant float kFogStart = 480.0;
constant float kFogEnd = 820.0;
constant float3 kFogColor = float3(0.0, 10.0 / 255.0, 15.0 / 255.0);
constant float kPairSeconds = 24.0;

float3 gibsonGradientLeft(int pair)
{
    if (pair == 0)
        return float3(0.38, 0.28, 1.0); /* violet */
    if (pair == 1)
        return float3(0.12, 0.42, 1.0); /* blue */
    if (pair == 2)
        return float3(0.12, 0.88, 0.38); /* green */
    return float3(1.0, 0.48, 0.10); /* orange */
}

float3 gibsonGradientRight(int pair)
{
    if (pair == 0)
        return float3(0.12, 0.78, 1.0); /* cyan */
    if (pair == 1)
        return float3(0.15, 0.95, 0.42); /* green */
    if (pair == 2)
        return float3(1.0, 0.52, 0.12); /* orange */
    return float3(0.55, 0.22, 1.0); /* violet */
}

struct GibsonGradEnds {
    float3 left;
    float3 right;
};

GibsonGradEnds gibsonGradientEnds(float time)
{
    const float t = time / kPairSeconds;
    const float idx = floor(t);
    const float frac = t - idx;
    const int i0 = int(idx) % 4;
    const int i1 = (i0 + 1) % 4;
    const float blend = smoothstep(0.72, 1.0, frac);
    GibsonGradEnds ends;
    ends.left = mix(gibsonGradientLeft(i0), gibsonGradientLeft(i1), blend);
    ends.right = mix(gibsonGradientRight(i0), gibsonGradientRight(i1), blend);
    return ends;
}

float3 gibsonTintedGlow(float3 tex, float3 viewPosition, float3 left, float3 right)
{
    float glow = max(max(tex.r, tex.g), tex.b);
    glow = pow(saturate(glow), 0.85);
    const float g = saturate(viewPosition.x / kGradientWidth + 0.5);
    float3 rgb = mix(left, right, g) * glow * 1.15;
    rgb = min(rgb, float3(1.0));
    const float fogFactor = saturate((kFogEnd - length(viewPosition)) / (kFogEnd - kFogStart));
    return mix(kFogColor, rgb, fogFactor);
}

vertex GibsonFloorVertexOut gibsonFloorVertex(
    GibsonFloorVertexIn in [[stage_in]],
    constant SCNSceneBuffer& scn_frame [[buffer(0)]],
    constant GibsonNodeBuffer& scn_node [[buffer(1)]])
{
    (void)scn_frame;
    GibsonFloorVertexOut out;
    const float4 pos = float4(in.position, 1.0);
    out.position = scn_node.modelViewProjectionTransform * pos;
    out.texcoord = in.texcoord;
    out.viewPosition = (scn_node.modelViewTransform * pos).xyz;
    return out;
}

fragment float4 gibsonFloorFragment(
    GibsonFloorVertexOut in [[stage_in]],
    constant SCNSceneBuffer& scn_frame [[buffer(0)]],
    texture2d<float, access::sample> diffuseTexture [[texture(0)]])
{
    (void)scn_frame;
    constexpr sampler circuitSampler(coord::normalized,
                                     address::repeat,
                                     filter::linear,
                                     mip_filter::linear);
    const float3 tex = diffuseTexture.sample(circuitSampler, in.texcoord).rgb;
    const float3 violet = float3(0.38, 0.28, 1.0);
    const float3 cyan = float3(0.12, 0.78, 1.0);
    const float3 rgb = gibsonTintedGlow(tex, in.viewPosition, violet, cyan);
    return float4(rgb, 1.0);
}

fragment float4 gibsonTowerFragment(
    GibsonFloorVertexOut in [[stage_in]],
    constant SCNSceneBuffer& scn_frame [[buffer(0)]],
    texture2d<float, access::sample> diffuseTexture [[texture(0)]])
{
    constexpr sampler glyphSampler(coord::normalized,
                                   address::clamp_to_edge,
                                   filter::linear,
                                   mip_filter::linear);
    const float4 tex = diffuseTexture.sample(glyphSampler, in.texcoord);
    const GibsonGradEnds ends = gibsonGradientEnds(scn_frame.time);
    const float3 rgb = gibsonTintedGlow(tex.rgb, in.viewPosition, ends.left, ends.right);
    return float4(rgb, tex.a);
}
