// build.rs
use wgsl_to_wgpu::{create_shader_module, MatrixVectorTypes, WriteOptions};

fn main() {
    println!("cargo:rerun-if-changed=src/shader.wgsl");

    // Read the shader source file.
    let wgsl_source = std::fs::read_to_string("src/shader.wgsl").unwrap();

    // Configure the output based on the dependencies for the project.
    let options = WriteOptions {
        derive_bytemuck_vertex: true,
        derive_encase_host_shareable: true,
        matrix_vector_types: MatrixVectorTypes::Glam,
        ..Default::default()
    };

    // Generate the bindings.
    let text = create_shader_module(&wgsl_source, "shader.wgsl", options).unwrap();
    std::fs::write("src/shader.rs", text.as_bytes()).unwrap();
}
