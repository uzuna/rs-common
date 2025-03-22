// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(1) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    // 頂点座標
    @location(0) position: vec3<f32>,
    // テクスチャ座標
    @location(1) tex_coords: vec2<f32>,
    // ノーマルマップ
    @location(2) normal: vec3<f32>,
}

// instance毎の回転・拡大・移動行列
// vertexにはmat4x4を直接渡せないので、4つのvec4<f32>で渡す
struct InstanceInput {
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
    instance: InstanceInput
) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    out.clip_position = camera.view_proj * model_matrix * vec4<f32>(model.position, 1.0);
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
 