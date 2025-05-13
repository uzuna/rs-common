//! gltfファイルを読み出して表示する

use std::path::PathBuf;

use eframe::egui_wgpu::{self, RenderState};
use fxhash::FxHashMap;
use wgpu_shader::{
    camera::{Camera, FollowCamera},
    graph::ModelNodeImpl,
    model,
    prelude::glam,
    rgltf::{PlNormal, PlNormalCameraBg, PlNormalMaterialBg, PlNormalModelBg},
    types,
    uniform::UniformBuffer,
    vertex::{VertexBuffer, VertexBufferSimple},
};

use crate::tf::{self, GraphBuilder};

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
    fn build_render_resources(&mut self, rs: &RenderState, aspect: f32) -> anyhow::Result<()> {
        // deviceを用いて各種リソースを作成する
        // wgpuリソースはrendererでデータ保持、UI操作系はViewAppで保持する
        let Some(graph) = self.graph.as_ref() else {
            return Err(anyhow::anyhow!("No graph found"));
        };
        let device = &rs.device;

        let mut rr = RenderResource::new(device, rs.target_format, aspect);

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
            rr.primitives.insert(*id, VertexImpl::Normal(buffer));
            rr.primitive_materials.insert(*id, p.material.clone());
        }

        for n in graph.graph.iter() {
            if let tf::GltfSlot::Draw(mesh) = n.value() {
                let material = rr
                    .primitive_materials
                    .get(mesh)
                    .ok_or(anyhow::anyhow!("No material found"))?;
                let trs = n.world();
                let model = UniformBuffer::new_encase(device, &types::uniform::Model::from(&trs));
                rr.add_draw_node(device, n.name(), *mesh, material.clone(), model);
            }
        }

        // グラフノードの特性として読み出したシーンの展開以外に、デフォルトカメラやデフォルトマテリアルを設定する必要がある
        // glTFで未定義の場合でも表示が行えるだけのデフォルト設定を作る必要がある。
        // 場合によってはこの情報自体も書き出せる必要がありそう。
        // 特定のフレームが書き出せることを想定にデータを構築する
        // 通常のゲームとの違いは、一定のモデルと一時的なデータの関係と違い
        // 認識情報が1フレームのみ有効な点。
        // モデルの移動に関しては、各フレームで変化があった(Name, Trs)を保持していれば良さそう -> KeyFrameの考え方
        rs.renderer.write().callback_resources.insert(rr);
        self.rframe = Some(RenderFrame::new());
        Ok(())
    }

    fn build_sample_render(&mut self, rs: &RenderState, aspect: f32) -> anyhow::Result<()> {
        let device = &rs.device;
        let mut rr = RenderResource::new(device, rs.target_format, aspect);

        let vert = model::CUBE
            .into_iter()
            .map(|x| types::vertex::Normal {
                position: glam::Vec3::new(x.x, x.y, x.z),
                normal: glam::Vec3::new(1.0, 0.0, 0.0),
            })
            .collect::<Vec<_>>();
        let index = model::CUBE_INDEX;
        let vb = VertexBuffer::new(device, &vert, &index);
        rr.primitives.insert(0, VertexImpl::Normal(vb));

        rr.add_material(
            device,
            "red",
            UniformBuffer::new_encase(
                device,
                &types::uniform::Material {
                    color: glam::Vec4::new(1.0, 0.0, 0.0, 1.0),
                    ..Default::default()
                },
            ),
        );

        let model =
            UniformBuffer::new_encase(device, &types::uniform::Model::from(&glam::Mat4::IDENTITY));
        rr.add_draw_node(device, "default", 0, "red".to_string(), model);

        rs.renderer.write().callback_resources.insert(rr);
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
                                    self.build_render_resources(wgpu_render_state, 1.0)
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
                    self.build_sample_render(frame.wgpu_render_state.as_ref().unwrap(), 1.0)
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

                    let (rect, _response) =
                        ui.allocate_exact_size(egui::Vec2::splat(300.0), egui::Sense::drag());

                    ui.painter()
                        .add(egui_wgpu::Callback::new_paint_callback(rect, *rframe));
                }
            });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Pipe {
    LineList,
    Opacity,
    Transparent,
}

enum VertexImpl<V> {
    Normal(VertexBuffer<V>),
    NormalSimple(VertexBufferSimple<V>),
}

impl<V> VertexImpl<V>
where
    V: bytemuck::Pod,
{
    fn set(&self, render_pass: &mut wgpu::RenderPass<'static>, slot: u32) {
        match self {
            VertexImpl::Normal(vb) => vb.set(render_pass, slot),
            VertexImpl::NormalSimple(vb) => vb.set(render_pass, slot),
        }
    }

    fn draw(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        match self {
            VertexImpl::Normal(vb) => render_pass.draw_indexed(0..vb.len(), 0, 0..1),
            VertexImpl::NormalSimple(vb) => render_pass.draw(0..vb.len(), 0..1),
        }
    }
}

pub struct RenderResource {
    // 描画パイプライン
    pipelines: FxHashMap<Pipe, PlNormal>,
    // カメラの操作計算インスタンス
    cams: FxHashMap<String, FollowCamera>,
    // カメラのUniformBuffer
    cambufs: FxHashMap<String, UniformBuffer<types::uniform::Camera>>,
    // カメラのBindGroup
    cambgs: FxHashMap<String, PlNormalCameraBg>,
    // マテリアルのUniformBuffer
    materials: FxHashMap<String, UniformBuffer<types::uniform::Material>>,
    materials_bg: FxHashMap<String, PlNormalMaterialBg>,
    // 描画するプリミティブの頂点データ
    primitives: FxHashMap<u32, VertexImpl<types::vertex::Normal>>,
    // プリミティブに関連付けられたデフォルトのマテリアルのキー
    primitive_materials: FxHashMap<u32, String>,
    // 描画するノードの情報
    nodes: FxHashMap<String, RenderSlot>,
}

impl RenderResource {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, aspect: f32) -> Self {
        // カメラ作成
        let mut cams = FxHashMap::default();
        let cam = FollowCamera::new(Camera::with_aspect(aspect));
        let cambuf = UniformBuffer::new_encase(device, &cam.camera().to_uniform());
        cams.insert("default".to_string(), cam);
        let mut cambufs = FxHashMap::default();
        cambufs.insert("default".to_string(), cambuf);

        let mut materials = FxHashMap::default();
        materials.insert(
            "default".to_string(),
            UniformBuffer::new_encase(device, &types::uniform::Material::default()),
        );
        let primitives = FxHashMap::default();
        let primitive_materials = FxHashMap::default();
        let mut pipelines = FxHashMap::default();
        pipelines.insert(
            Pipe::LineList,
            PlNormal::new(
                device,
                format,
                wgpu::PrimitiveTopology::LineList,
                wgpu_shader::prelude::Blend::Replace,
            ),
        );
        pipelines.insert(
            Pipe::Opacity,
            PlNormal::new(
                device,
                format,
                wgpu::PrimitiveTopology::TriangleList,
                wgpu_shader::prelude::Blend::Replace,
            ),
        );
        pipelines.insert(
            Pipe::Transparent,
            PlNormal::new(
                device,
                format,
                wgpu::PrimitiveTopology::TriangleList,
                wgpu_shader::prelude::Blend::Alpha,
            ),
        );

        let mut cambgs = FxHashMap::default();
        for (i, cam) in cambufs.iter() {
            let cam_bg = PlNormal::camera_bg(device, cam.buffer());
            cambgs.insert(i.clone(), cam_bg);
        }

        let mut materials_bg = FxHashMap::default();
        for (i, mat) in materials.iter() {
            let mat_bg = PlNormal::material_bg(device, mat.buffer());
            materials_bg.insert(i.clone(), mat_bg);
        }

        let nodes = FxHashMap::default();
        Self {
            pipelines,
            cams,
            cambufs,
            cambgs,
            materials,
            materials_bg,
            primitives,
            primitive_materials,
            nodes,
        }
    }

    pub fn add_material(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        buffer: UniformBuffer<types::uniform::Material>,
    ) {
        let bg = PlNormal::material_bg(device, buffer.buffer());
        self.materials.insert(name.to_string(), buffer);
        self.materials_bg.insert(name.to_string(), bg);
    }

    pub fn add_camera(&mut self, name: &str, buffer: UniformBuffer<types::uniform::Camera>) {
        self.cambufs.insert(name.to_string(), buffer);
    }

    pub fn add_node(&mut self, name: &str, v: RenderSlot) {
        self.nodes.insert(name.to_string(), v);
    }

    pub fn add_draw_node(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        primitive: u32,
        material: String,
        model: UniformBuffer<types::uniform::Model>,
    ) {
        let bg = PlNormal::model_bg(device, model.buffer());
        let obj = DrawObject {
            model,
            bg,
            primitive,
            material,
        };
        self.nodes
            .insert(name.to_string(), RenderSlot::Opacity(obj));
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        let cambg = self.cambgs.get("default").expect("No default camera found");
        for (_name, node) in self.nodes.iter() {
            if let RenderSlot::Opacity(obj) = node {
                let pipe = self.pipelines.get(&Pipe::Opacity).unwrap();
                render_pass.set_pipeline(pipe.pipeline());
                cambg.set(render_pass);
                obj.bg.set(render_pass);
                let mat = self
                    .materials_bg
                    .get(&obj.material)
                    .unwrap_or_else(|| panic!("No material found: {}", obj.material));
                mat.set(render_pass);
                let mesh = self
                    .primitives
                    .get(&obj.primitive)
                    .expect("No primitive found");
                mesh.set(render_pass, 0);
                mesh.draw(render_pass);
            }
        }
    }
}

enum RenderSlot {
    None,
    Opacity(DrawObject),
    Transparent(DrawObject),
}

impl PartialEq for RenderSlot {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (RenderSlot::None, RenderSlot::None)
                | (RenderSlot::Opacity(_), RenderSlot::Opacity(_))
                | (RenderSlot::Transparent(_), RenderSlot::Transparent(_))
        )
    }
}

impl PartialOrd for RenderSlot {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            // 透明度のあるものは常に後に描画する
            (RenderSlot::Transparent(_), _) => Some(std::cmp::Ordering::Greater),
            (_, _) => Some(std::cmp::Ordering::Equal),
        }
    }
}

struct DrawObject {
    model: UniformBuffer<types::uniform::Model>,
    bg: PlNormalModelBg,
    primitive: u32,
    material: String,
}

/// レンダリング更新時にデータを配置するための型
/// 常に使い捨てる情報となっている
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderFrame {}

impl RenderFrame {
    pub fn new() -> Self {
        Self {}
    }
}

impl egui_wgpu::CallbackTrait for RenderFrame {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &RenderResource = resources.get().unwrap();
        resources.paint(render_pass);
    }

    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        Vec::new()
    }
}
