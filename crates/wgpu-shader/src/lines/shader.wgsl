// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;


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


// 頂点シェーダーを宣言
@vertex
fn vs_main(
    // 頂点バッファの入力
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    // 頂点の色をそのまま出力
    out.color = model.color;
    // カメラの射影行列を使ってクリップ座標に変換
    out.clip_position = camera.view_proj * model.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
