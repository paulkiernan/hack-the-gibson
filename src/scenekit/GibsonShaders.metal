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
    float glow = max(max(tex.r, tex.g), tex.b);
    glow = pow(saturate(glow), 0.85);

    /* View-space X: purple/blue on the left of the frame, cyan on the right. */
    const float t = saturate(in.viewPosition.x / kGradientWidth + 0.5);
    const float3 violet = float3(0.38, 0.28, 1.0);
    const float3 cyan = float3(0.12, 0.78, 1.0);
    const float3 col = mix(violet, cyan, t);
    float3 rgb = col * glow * 1.15;
    rgb = min(rgb, float3(1.0));

    const float fogFactor = saturate((kFogEnd - length(in.viewPosition)) / (kFogEnd - kFogStart));
    rgb = mix(kFogColor, rgb, fogFactor);

    return float4(rgb, 1.0);
}
