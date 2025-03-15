struct VertexInput {
    // 頂点位置
    @location(0) position: vec3<f32>,
    // 頂点に対応する色
    @location(1) color: vec3<f32>,
};

// 同様
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

// 頂点シェーダーを宣言
@vertex
fn vs_main(
    // 頂点バッファの入力
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.color = model.color;
    out.clip_position = vec4<f32>(model.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
