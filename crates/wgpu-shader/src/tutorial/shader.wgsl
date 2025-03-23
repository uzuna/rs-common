// カメラの射影行列
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
};
@group(1) @binding(0)
var<uniform> camera: Camera;

// ライト
struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
}
@group(2) @binding(0)
var<uniform> light: Light;

struct VertexInput {
    // 頂点座標
    @location(0) position: vec3<f32>,
    // テクスチャ座標
    @location(1) tex_coords: vec2<f32>,
    // ノーマルマップ
    @location(2) normal: vec3<f32>,
}

struct InstanceInput {
    // instance毎の回転・拡大・移動行列
    // vertexにはmat4x4を直接渡せないので、4つのvec4<f32>で渡す
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
    // 法線マトリックス
    // こちらもvec3<f32>で渡す
    // @location(9) normal_matrix_0: vec3<f32>,
    // @location(10) normal_matrix_1: vec3<f32>,
    // @location(11) normal_matrix_2: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
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
    // let normal_matrix = mat3x3<f32>(
    //     instance.normal_matrix_0,
    //     instance.normal_matrix_1,
    //     instance.normal_matrix_2,
    // );
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    // 法線も移動・回転に合わせて変換
    // out.world_normal = normal_matrix * model.normal;
    out.world_normal = model.normal;
    // 頂点座標の移動・回転
    var world_position: vec4<f32> = model_matrix * vec4<f32>(model.position, 1.0);
    // 光源計算のためにワールド座標を渡す
    out.world_position = world_position.xyz;
    // カメラの射影行列を使ってクリップ座標に変換
    out.clip_position = camera.view_proj * world_position;
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
    let object_color: vec4<f32> = textureSample(t_diffuse, s_diffuse, in.tex_coords);

    // 光源からのベクトルを計算、面の露光量を計算して色の強さを反映
    let light_dir = normalize(light.position - in.world_position);
    let diffuse_strength = max(dot(in.world_normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;
 
    // アンビエントカラーを乗算
    // 物体が環境光の影響を受けた色になる
    let ambient_strength = 0.1;
    let ambient_color = light.color * ambient_strength;

    let result = (ambient_color + diffuse_color) * object_color.xyz;

    return vec4<f32>(result, object_color.a);
}
 