use std::path::{Path, PathBuf};

use glob::glob;
use wgsl_to_wgpu::{create_shader_module, MatrixVectorTypes, WriteOptions};

fn main() {
    for entry in glob("src/**/*.wgsl").unwrap() {
        if let Ok(entry) = entry {
            generate(entry.to_str().unwrap());
        }
    }
}

fn generate(path: &str) {
    println!("cargo:rerun-if-changed={path}");

    // Read the shader source file.
    let wgsl_source = std::fs::read_to_string(path).unwrap();

    // Configure the output based on the dependencies for the project.
    let options = WriteOptions {
        derive_bytemuck_vertex: true,
        derive_encase_host_shareable: true,
        matrix_vector_types: MatrixVectorTypes::Glam,
        ..Default::default()
    };
    let name = Path::new(path).file_name().unwrap().to_str().unwrap();
    let mut p = PathBuf::from(path);
    p.set_extension("rs");

    // Generate the bindings.
    let text = create_shader_module(&wgsl_source, name, options).unwrap();
    std::fs::write(&p, text.as_bytes()).unwrap();
}
