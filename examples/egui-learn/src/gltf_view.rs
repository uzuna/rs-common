//! gltfファイルを読み出して表示する

use std::path::PathBuf;

use eframe::egui_wgpu::{self, RenderState};
use wgpu_shader::{
    graph::ModelNodeImpl, rgltf::PlNormal, types, uniform::UniformBuffer, vertex::VertexBuffer,
};

use crate::{
    render::{
        sample, CameraUpdateRequest, PipeType, RenderFrame, RenderResource, SceneResource,
        VertexWrap,
    },
    tf::{self, GraphBuilder},
    ui::move_camera_by_pointer,
};

pub struct ViewApp {
    loaded: Option<PathBuf>,
    error: Option<String>,
    graph: Option<GraphBuilder>,
    rframe: Option<RenderFrame>,
}

impl ViewApp {
    pub fn new() -> Self {
        Self {
            loaded: None,
            error: None,
            graph: None,
            rframe: None,
        }
    }

    // glTFの読み出し結果をGPUリソースに反映する
    fn build_render_resources(&mut self, rs: &RenderState) -> anyhow::Result<()> {
        // deviceを用いて各種リソースを作成する
        // wgpuリソースはrendererでデータ保持、UI操作系はViewAppで保持する
        let Some(graph) = self.graph.as_ref() else {
            return Err(anyhow::anyhow!("No graph found"));
        };
        let device = &rs.device;

        let mut rr = {
            if let Some(s) = rs.renderer.read().callback_resources.get::<SceneResource>() {
                RenderResource::<PlNormal>::new(device, rs.target_format, s)
            } else {
                return Err(anyhow::anyhow!("No scene resource found"));
            }
        };

        // マテリアルとプリミティブを設定
        for (name, material) in &graph.materials {
            let buffer = UniformBuffer::new_encase(
                device,
                &types::uniform::Material::from(material.clone()),
            );
            rr.add_material(device, name, buffer);
        }

        for (id, mesh) in &graph.meshes {
            // 現時点では1つのプリミティブにしか対応しない
            let p = mesh
                .primitives
                .first()
                .ok_or(anyhow::anyhow!("No primitive found"))?;
            let buffer = p.try_to_normal()?;
            let index = p.index.as_ref().ok_or(anyhow::anyhow!("No index found"))?;
            let buffer = VertexBuffer::new(device, &buffer, index);
            rr.add_primitive(*id, VertexWrap::Indexed(buffer), p.material.clone());
        }

        // グラフノードの情報を元に描画リストを作成
        for n in graph.graph.iter() {
            if let tf::GltfSlot::Draw(mesh) = n.value() {
                let material_name = rr
                    .get_material_name(mesh)
                    .ok_or(anyhow::anyhow!("No material found"))?;
                let trs = n.world();
                let model = UniformBuffer::new_encase(device, &types::uniform::Model::from(&trs));
                rr.add_draw_node(
                    device,
                    n.name(),
                    PipeType::Opacity,
                    *mesh,
                    material_name.clone(),
                    model,
                );
            }
        }

        rs.renderer.write().callback_resources.insert(rr);
        self.rframe = Some(RenderFrame::new());
        Ok(())
    }

    fn build_sample_render(&mut self, rs: &RenderState) -> anyhow::Result<()> {
        sample(rs);
        self.rframe = Some(RenderFrame::new());
        Ok(())
    }
}

impl eframe::App for ViewApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
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

                                // wgpu_shaderのリソースを作る
                                if let Some(wgpu_render_state) = frame.wgpu_render_state.as_ref() {
                                    self.build_render_resources(wgpu_render_state)
                                        .expect("Failed to build render resources");
                                }
                            }
                            Err(e) => {
                                self.error = Some(format!("Failed to load file: {}", e));
                            }
                        }
                    }
                }
                if ui.button("load default").clicked() {
                    self.build_sample_render(frame.wgpu_render_state.as_ref().unwrap())
                        .expect("Failed to load default");
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
                if let Some(rframe) = &self.rframe {
                    ui.label("Render frame:");

                    let (rect, response) =
                        ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());

                    if let Some(prop) = move_camera_by_pointer(ui, response) {
                        let mut rframe = rframe.clone();
                        rframe.camera_update = Some(CameraUpdateRequest::new(
                            SceneResource::DEFAULT_CAMERA,
                            prop,
                        ));
                        ui.painter()
                            .add(egui_wgpu::Callback::new_paint_callback(rect, rframe));
                    } else {
                        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                            rect,
                            rframe.clone(),
                        ));
                    }
                }
            });
    }
}
