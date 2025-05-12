const PI: f32 = 3.141592653589793;

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
    @location(1) metallic: f32,
    @location(2) roughness: f32,
}
@group(2) @binding(0)
var<uniform> material: Material;

// 頂点入力
struct VertexInput{
    // 頂点位置
    @location(0) position: vec3<f32>,
    // ノーマル
    @location(1) normal: vec3<f32>,
}

// 頂点出力
struct VertexOutput{
    @builtin(position) position: vec4<f32>,
    // 回転後のノーマルベクトル
    @location(0) world_normal: vec3<f32>,
    // 光反射計算用のワールド座標
    @location(1) world_position: vec3<f32>,
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

    // 頂点の位置をワールド座標に変換
    var world_position: vec4<f32> = model_matrix * vert.position;
    out.world_normal = normalize((normal_matrix * vec4<f32>(vert.normal, 0.0)).xyz);

    // カメラの射影行列を使ってクリップ座標に変換
    out.position = camera.view_proj * world_position;
    // ワールド座標を出力
    out.world_position = world_position.xyz;
    return out;
}


// reference from: https://github.com/JiyuHuang/webgpu-gltf-viewer/blob/main/src/shaders/frag.wgsl.ts
// 反射光の角度分布特性に基づく光学シミュレーション
// https://zenn.dev/mebiusbox/books/619c81d2fbeafd/viewer/77aea9 に解説がある
// metallic: 金属度
// roughness: 粗さ
// l: 光源の方向ベクトル
// v: 視線の方向ベクトル
// n: 法線ベクトル
fn brdf(color: vec3<f32>,
        metallic: f32,
        roughness: f32,
        l: vec3<f32>,
        v: vec3<f32>,
        n: vec3<f32>) -> vec3<f32>
{
    // h: half vector = 光線と視線の中間ベクトル
    let h = normalize(l + v);
    let ndotl = clamp(dot(n, l), 0.0, 1.0);
    let ndotv = abs(dot(n, v));
    let ndoth = clamp(dot(n, h), 0.0, 1.0);
    let vdoth = clamp(dot(v, h), 0.0, 1.0);

    // フレネル反射率。拡散反射率と鏡面反射率の比率を決定する
    // 金属度が高いと拡散反射色は小さく、鏡面反射色は大きくなる
    let f0 = vec3<f32>(0.04);
    let diffuseColor = color * (1.0 - f0) * (1.0 - metallic);
    let specularColor = mix(f0, color, metallic);
    
    // Calculate the shading terms for the microfacet specular shading model
    // The following equation models the Fresnel reflectance term of the spec equation (aka F())
    let reflectance = max(max(specularColor.r, specularColor.g), specularColor.b);
    let reflectance0 = specularColor;
    let reflectance9 = vec3<f32>(clamp(reflectance * 25.0, 0.0, 1.0));
    let f = reflectance0 + (reflectance9 - reflectance0) * pow(1.0 - vdoth, 5.0);

    // 幾何減衰項の計算。粗さを表現するときのモデルの一種
    // geometric Occlusion
    // This calculates the specular geometric attenuation (aka G()),
    // where rougher material will reflect less light back to the viewer.
    let r2 = roughness * roughness;
    let r4 = r2 * r2;
    let attenuationL = 2.0 * ndotl / (ndotl + sqrt(r4 + (1.0 - r4) * ndotl * ndotl));
    let attenuationV = 2.0 * ndotv / (ndotv + sqrt(r4 + (1.0 - r4) * ndotv * ndotv));
    let g = attenuationL * attenuationV;

    // microfacet distribution
    // The following equation(s) model the distribution of microfacet normals across the area being drawn (aka D())
    // Implementation from "Average Irregularity Representation of a Roughened Surface for Ray Reflection" by T. S. Trowbridge, and K. P. Reitz
    // Follows the distribution function recommended in the SIGGRAPH 2013 course notes from EPIC Games [1], Equation 3.
    let temp = ndoth * ndoth * (r2 - 1.0) + 1.0;
    let d = r2 / (PI * temp * temp);

    let diffuse = (1.0 - f) / PI * diffuseColor;
    let specular = max(f * g * d / (4.0 * ndotl * ndotv), vec3<f32>(0.0));
    return ndotl * (diffuse + specular) * 2.0 + color * 0.1;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // TODO: Ambient Lightの設定
    var color = material.color;
    // TODO: brdfの計算
    // aoはocculusion mapで環境光を調整するためのもの
    // emissiveは自発光の強度
    // var rgb = brdf(color.rgb, metallic, roughness, lightDir, viewDir, normal) * ao + emissive;
    // rgb = pow(rgb, vec3<f32>(1.0 / 2.2));
    // return vec4<f32>(rgb, color.a);
    return color;
}
