// 光源位置確認デバッグ向けのシェーダー

// 使用しているカメラの情報と合わせるためにカメラの情報を定義
struct Camera {
    // カメラの位置
    view_pos: vec4<f32>,
    // カメラの射影行列
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: Camera;

// ライト
struct Light {
    // ライトの位置
    position: vec3<f32>,
    // ライトの色
    color: vec3<f32>,
}
@group(1) @binding(0)
var<uniform> light: Light;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(
    model: VertexInput,
) -> VertexOutput {
    let scale = 0.25;
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(model.position * scale + light.position, 1.0);
    out.color = light.color;
    return out;
}

// Fragment shader

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
