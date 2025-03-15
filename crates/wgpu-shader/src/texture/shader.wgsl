struct VertexInput {
    // 頂点座標
    @location(0) position: vec3<f32>,
    // テクスチャ座標
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    out.clip_position = vec4<f32>(model.position, 1.0);
    return out;
}

// テクスチャリソース
@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
// テクスチャサンプラー
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // サンプラーを使って座標に対応する色を取得
    return textureSample(t_diffuse, s_diffuse, in.tex_coords);
}
 