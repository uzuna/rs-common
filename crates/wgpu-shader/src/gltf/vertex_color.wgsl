// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: Camera;

// モデルごとの頂点位置調整
struct Model{
    // 頂点の変換行列
    @location(0) matrix: mat4x4<f32>,
    // ノーマルマップ変更行列
    @location(1) normal: mat4x4<f32>,
}
@group(1) @binding(0)
var<uniform> model: Model;

struct Material{
    // オブジェクトの色補正
    @location(0) color: vec4<f32>,
}
@group(2) @binding(0)
var<uniform> material: Material;

// 頂点入力
struct VertexInput{
    // 頂点位置
    @location(0) position: vec3<f32>,
    // ノーマル
    @location(1) normal: vec3<f32>,
    // 頂点に対応する色: Pod deriveの関係で、vec3とする
    // 一部だけ透明という使い方をしないという想定
    @location(2) color: vec3<f32>,
}

// 頂点出力
struct VertexOutput{
    @builtin(position) position: vec4<f32>,
    // 回転後のノーマルベクトル
    @location(0) world_normal: vec3<f32>,
    // 光反射計算用のワールド座標
    @location(1) world_position: vec3<f32>,
    // 頂点カラー
    @location(2) color: vec4<f32>,
}

// 頂点シェーダーを宣言
@vertex
fn vs_main(
    // 頂点バッファの入力
    vert: VertexInput,
) -> VertexOutput {
    // instance毎の回転・拡大・移動行列
    let model_matrix = model.matrix;
    let normal_matrix = model.normal;

    var out: VertexOutput;

    // 頂点の色をそのまま出力
    out.color = vec4<f32>(vert.color, 1.0);
    // 頂点の位置をワールド座標に変換
    var world_position: vec4<f32> = model_matrix * vert.position;
    out.world_normal = normalize((normal_matrix * vec4<f32>(vert.normal, 0.0)).xyz);

    // カメラの射影行列を使ってクリップ座標に変換
    out.position = camera.view_proj * world_position;
    // ワールド座標を出力
    out.world_position = world_position.xyz;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = material.color * vec4<f32>(in.color.rgb, 1.0);
    return color;
}
