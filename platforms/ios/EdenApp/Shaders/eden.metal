//
//  eden.metal
//  Eden
//
//  Fullscreen triangle sampling the 1024x640 RGBA8 stage texture. The host
//  aspect-fits the viewport, so no per-frame uniform buffer is needed.
//

#include <metal_stdlib>

using namespace metal;

struct EdenVertexOut {
    float4 position [[position]];
    float2 uv;
};

vertex EdenVertexOut eden_vertex(uint vertexID [[vertex_id]])
{
    const float2 positions[3] = {
        float2(-1.0, -1.0),
        float2( 3.0, -1.0),
        float2(-1.0,  3.0),
    };
    const float2 uvs[3] = {
        float2(0.0, 1.0),
        float2(2.0, 1.0),
        float2(0.0, -1.0),
    };

    EdenVertexOut out;
    out.position = float4(positions[vertexID], 0.0, 1.0);
    out.uv = uvs[vertexID];
    return out;
}

fragment float4 eden_fragment(EdenVertexOut in [[stage_in]],
                              texture2d<float> stage [[texture(0)]],
                              sampler linearSampler [[sampler(0)]])
{
    return stage.sample(linearSampler, in.uv);
}
