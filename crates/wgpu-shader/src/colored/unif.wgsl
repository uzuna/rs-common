// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

struct GlobalInfo{
    // グローバルTRS行列。ワールド空間中の移動や回転。インスタンスの移動など主
    matrix: mat4x4<f32>,
}
// オブジェクトグループ毎に設定する
@group(1) @binding(0)
var<uniform> global_info: GlobalInfo;

struct DrawInfo{
    // ローカルTRS行列。移動後のローカル基準座標に対する移動や回転。インスタンス内のパーツの移動など
    matrix: mat4x4<f32>,
    // オブジェクトの色補正
    color: vec4<f32>,
}
// オブジェクトごとに変更する場合があるのでカメラとは別のバインディングにする
@group(2) @binding(0)
var<uniform> draw_info: DrawInfo;

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
    model: VertexInput
) -> VertexOutput {
    // instance毎の回転・拡大・移動行列
    let model_matrix = global_info.matrix * draw_info.matrix;

    var out: VertexOutput;

    // 頂点の色をそのまま出力
    out.color = model.color;
    // 頂点の位置をワールド座標に変換
    var world_position: vec4<f32> = model_matrix * model.position;
    // カメラの射影行列を使ってクリップ座標に変換
    out.clip_position = camera.view_proj * world_position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color * draw_info.color;
}
