// Vertex shader
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
};

struct Window {
    resolution: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uw: Window;

@vertex
fn vs_main(
    particle: VertexInput,
    @builtin(vertex_index) vNdx: u32,
) -> VertexOutput {
    let points = array(
        vec3f(-1, -1, 0),
        vec3f( 1, -1, 0),
        vec3f(-1,  1, 0),
        vec3f(-1,  1, 0),
        vec3f( 1, -1, 0),
        vec3f( 1,  1, 0),
    );

    let resolution = uw.resolution.xyz;

    var out: VertexOutput;  
    let pos = points[vNdx];
    out.pos = vec4<f32>(particle.position + pos * 12.0 / resolution, 1.0);
    out.color = particle.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
