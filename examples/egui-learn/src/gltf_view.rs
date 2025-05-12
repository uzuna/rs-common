//! gltfファイルを読み出して表示する

use std::path::PathBuf;

use wgpu_shader::graph::ModelNodeImpl;

use crate::tf::{self, GraphBuilder};

pub struct ViewApp {
    loaded: Option<PathBuf>,
    error: Option<String>,
    graph: Option<GraphBuilder>,
}

impl ViewApp {
    pub fn new() -> Self {
        Self {
            loaded: None,
            error: None,
            graph: None,
        }
    }
}

impl eframe::App for ViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ウィンドウを作ってファイルを選択する
        egui::Window::new("glTF Viewer")
            .default_width(300.0)
            .default_height(300.0)
            .show(ctx, |ui| {
                if ui.button("Open file…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        match path.extension() {
                            Some(ext) if ext == "gltf" || ext == "glb" => {}
                            _ => {
                                ui.label("Not a glTF file");
                                return;
                            }
                        }
                        match tf::load(&path) {
                            Ok(builder) => {
                                self.loaded = Some(path);
                                self.graph = Some(builder);
                                self.error = None;
                            }
                            Err(e) => {
                                self.error = Some(format!("Failed to load file: {}", e));
                            }
                        }
                    }
                }
                if let Some(path) = &self.loaded {
                    ui.label(format!("Loaded: {}", path.display()));
                }
                if let Some(error) = &self.error {
                    ui.label(format!("Error: {}", error));
                }
                if let Some(graph) = &self.graph {
                    for node in graph.graph.iter() {
                        ui.label(format!("Node: {} {}", node.name(), node.value()));
                    }
                }
            });
    }
}
