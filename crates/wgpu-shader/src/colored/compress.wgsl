// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

/// 座標補正用の構造体
struct Compression{
    // 頂点位置
    @location(0) position: vec4<f32>,
}

@group(1) @binding(0)
var<uniform> comp: Compression;

struct VertexInput{
    // 頂点位置+padding
    @location(0) position: vec4<f32>,
    // 頂点に対応する色
    @location(1) color: vec4<f32>,
}

struct VertexOutput{
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}


struct InstanceInput {
    // instance毎の回転・拡大・移動行列
    // vertexにはmat4x4を直接渡せないので、4つのvec4<f32>で渡す
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
}

// 頂点シェーダーを宣言
@vertex
fn vs_main(
    // 頂点バッファの入力
    model: VertexInput,
    instance: InstanceInput
) -> VertexOutput {
    // instance毎の回転・拡大・移動行列
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    var out: VertexOutput;

    // 頂点の色をそのまま出力
    out.color = model.color;
    // 頂点の位置をワールド座標に変換
    var world_position: vec4<f32> = model_matrix * model.position;
    var comp_posision: vec4<f32> = comp.position * world_position;
    // カメラの射影行列を使ってクリップ座標に変換
    out.clip_position = camera.view_proj * comp_posision;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
