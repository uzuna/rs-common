//! gltfファイルを読み出して表示する

use std::path::PathBuf;

use eframe::egui_wgpu::{self, RenderState};
use fxhash::FxHashMap;
use wgpu::PrimitiveTopology;
use wgpu_shader::{
    camera::{Camera, ControlProperty, FollowCamera},
    constraint::{BindGroupImpl, PipelineConstraint},
    graph::ModelNodeImpl,
    model,
    prelude::{glam, Blend},
    rgltf::PlNormal,
    types,
    uniform::UniformBuffer,
    vertex::{VertexBuffer, VertexBufferSimple},
};

use crate::{
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
    pub fn new(rs: &RenderState, aspect: f32) -> Self {
        let mut s = Self {
            loaded: None,
            error: None,
            graph: None,
            rframe: None,
        };
        s.init_camera(rs, aspect);
        s
    }

    fn init_camera(&mut self, rs: &RenderState, aspect: f32) {
        // カメラ作成がまだの場合は作成する
        if rs
            .renderer
            .read()
            .callback_resources
            .get::<SceneResource>()
            .is_none()
        {
            let sc = SceneResource::new(&rs.device, aspect);
            rs.renderer.write().callback_resources.insert(sc);
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
            rr.primitives.insert(*id, VertexImpl::Normal(buffer));
            rr.primitive_materials.insert(*id, p.material.clone());
        }

        // グラフノードの情報を元に描画リストを作成
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

        rs.renderer.write().callback_resources.insert(rr);
        self.rframe = Some(RenderFrame::new());
        Ok(())
    }

    fn build_sample_render(&mut self, rs: &RenderState) -> anyhow::Result<()> {
        let device = &rs.device;
        let mut rr = {
            if let Some(s) = rs.renderer.read().callback_resources.get::<SceneResource>() {
                RenderResource::<PlNormal>::new(device, rs.target_format, s)
            } else {
                return Err(anyhow::anyhow!("No scene resource found"));
            }
        };

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

/// シーン全体で共通のリソース
pub struct SceneResource {
    // 名前 -> ID変換
    names: FxHashMap<String, u32>,
    // カメラの操作計算インスタンス
    cams: FxHashMap<u32, FollowCamera>,
    // カメラのUniformBuffer
    cambufs: FxHashMap<u32, UniformBuffer<types::uniform::Camera>>,
}

impl SceneResource {
    pub const DEFAULT_CAMERA: u32 = 0;

    pub fn new(device: &wgpu::Device, aspect: f32) -> Self {
        let mut s = Self {
            names: FxHashMap::default(),
            cams: FxHashMap::default(),
            cambufs: FxHashMap::default(),
        };
        s.init(device, aspect);
        s
    }

    // デフォルトカメラの設定
    fn init(&mut self, device: &wgpu::Device, aspect: f32) {
        let cam = FollowCamera::new(Camera::with_aspect(aspect));
        self.add_camera(device, "default", cam);
    }

    /// カメラの追加
    fn add_camera(&mut self, device: &wgpu::Device, name: impl Into<String>, cam: FollowCamera) {
        let id = self.names.len() as u32;
        self.names.insert(name.into(), id);
        self.cams.insert(id, cam);
        let buffer = UniformBuffer::new_encase(device, &self.cams[&id].camera().to_uniform());
        self.cambufs.insert(id, buffer);
    }

    /// カメラの移動
    fn update(&mut self, queue: &wgpu::Queue, req: &CameraUpdateRequest) {
        if let Some(cam) = self.cams.get_mut(&req.cam) {
            cam.update_by_property(&req.prop, false);
            if let Some(buf) = self.cambufs.get_mut(&req.cam) {
                queue.write_buffer(
                    buf.buffer(),
                    0,
                    bytemuck::cast_slice(&[cam.camera().to_uniform()]),
                );
            }
        }
    }

    // カメラバッファリストの取得
    fn buffers(&self) -> impl Iterator<Item = (u32, &UniformBuffer<types::uniform::Camera>)> {
        self.cambufs.iter().map(|(k, v)| (*k, v))
    }
}

pub struct RenderResource<P>
where
    P: PipelineConstraint,
{
    // 描画パイプライン
    pipelines: FxHashMap<Pipe, P>,
    // カメラのBindGroup
    cambgs: FxHashMap<u32, P::CameraBg>,
    // マテリアルのUniformBuffer
    materials: FxHashMap<String, UniformBuffer<P::Material>>,
    materials_bg: FxHashMap<String, P::MaterialBg>,
    // 描画するプリミティブの頂点データ
    primitives: FxHashMap<u32, VertexImpl<P::Vertex>>,
    // プリミティブに関連付けられたデフォルトのマテリアルのキー
    primitive_materials: FxHashMap<u32, String>,
    // 描画するノードの情報
    nodes: FxHashMap<String, RenderSlot<P::ModelBg>>,
}

impl<P> RenderResource<P>
where
    P: PipelineConstraint,
    P::Material: encase::ShaderType + encase::internal::WriteInto,
{
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, scenes: &SceneResource) -> Self {
        let mut materials = FxHashMap::default();
        materials.insert(
            "default".to_string(),
            UniformBuffer::new_encase(device, &P::default_material()),
        );
        let primitives = FxHashMap::default();
        let primitive_materials = FxHashMap::default();
        let params = [
            (Pipe::LineList, PrimitiveTopology::LineList, Blend::Replace),
            (
                Pipe::Opacity,
                PrimitiveTopology::TriangleList,
                Blend::Replace,
            ),
            (
                Pipe::Transparent,
                PrimitiveTopology::TriangleList,
                Blend::Alpha,
            ),
        ];
        let mut pipelines = FxHashMap::default();
        for (pipe, topology, blend) in params.iter() {
            let pipeline = P::new_pipeline(device, format, *topology, *blend);
            pipelines.insert(*pipe, pipeline);
        }

        let mut cambgs = FxHashMap::default();
        for (i, cam) in scenes.buffers() {
            let cam_bg = P::camera_bg(device, cam.buffer());
            cambgs.insert(i, cam_bg);
        }

        let mut materials_bg = FxHashMap::default();
        for (i, mat) in materials.iter() {
            let mat_bg = P::material_bg(device, mat.buffer());
            materials_bg.insert(i.clone(), mat_bg);
        }

        let nodes = FxHashMap::default();
        Self {
            pipelines,
            cambgs,
            materials,
            materials_bg,
            primitives,
            primitive_materials,
            nodes,
        }
    }
}

impl<P> RenderResource<P>
where
    P: PipelineConstraint,
{
    /// マテリアルの追加
    pub fn add_material(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        buffer: UniformBuffer<P::Material>,
    ) {
        let bg = P::material_bg(device, buffer.buffer());
        self.materials.insert(name.to_string(), buffer);
        self.materials_bg.insert(name.to_string(), bg);
    }

    /// 描画ノードの追加
    pub fn add_draw_node(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        primitive: u32,
        material: String,
        model: UniformBuffer<types::uniform::Model>,
    ) {
        let bg = P::model_bg(device, model.buffer());
        let obj = DrawObject {
            model,
            bg,
            primitive,
            material,
        };
        self.nodes
            .insert(name.to_string(), RenderSlot::Opacity(obj));
    }

    /// 新しいカメラの追加
    #[allow(dead_code)]
    fn add_camera(
        &mut self,
        device: &wgpu::Device,
        cam_id: u32,
        cam: &UniformBuffer<types::uniform::Model>,
    ) {
        let bg: P::CameraBg = P::camera_bg(device, cam.buffer());
        self.cambgs.insert(cam_id, bg);
    }
}

impl<P> RenderResource<P>
where
    P: PipelineConstraint,
    P::CameraBg: BindGroupImpl,
    P::ModelBg: BindGroupImpl,
    P::MaterialBg: BindGroupImpl,
    P::Vertex: bytemuck::Pod,
{
    fn paint(&self, render_pass: &mut wgpu::RenderPass<'static>, cam_id: u32) {
        let cambg = self.cambgs.get(&cam_id).expect("No default camera found");
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
                mesh.set(render_pass, P::vertex_slot());
                mesh.draw(render_pass);
            }
        }
    }
}

enum RenderSlot<Mbg> {
    None,
    Opacity(DrawObject<Mbg>),
    Transparent(DrawObject<Mbg>),
}

impl<Mbg> PartialEq for RenderSlot<Mbg> {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (RenderSlot::None, RenderSlot::None)
                | (RenderSlot::Opacity(_), RenderSlot::Opacity(_))
                | (RenderSlot::Transparent(_), RenderSlot::Transparent(_))
        )
    }
}

impl<Mbg> PartialOrd for RenderSlot<Mbg> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            // 透明度のあるものは常に後に描画する
            (RenderSlot::Transparent(_), _) => Some(std::cmp::Ordering::Greater),
            (_, _) => Some(std::cmp::Ordering::Equal),
        }
    }
}

struct DrawObject<Mbg> {
    model: UniformBuffer<types::uniform::Model>,
    bg: Mbg,
    primitive: u32,
    material: String,
}

#[derive(Debug, Clone)]
struct CameraUpdateRequest {
    cam: u32,
    prop: ControlProperty,
}

impl CameraUpdateRequest {
    pub fn new(cam: u32, prop: ControlProperty) -> Self {
        Self { cam, prop }
    }
}

/// レンダリング更新時にデータを配置するための型
/// 常に使い捨てる情報となっている
#[derive(Debug, Clone)]
pub struct RenderFrame {
    camera_update: Option<CameraUpdateRequest>,
}

impl RenderFrame {
    pub fn new() -> Self {
        Self {
            camera_update: None,
        }
    }
}

impl egui_wgpu::CallbackTrait for RenderFrame {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let resources: &RenderResource<PlNormal> = resources.get().unwrap();
        resources.paint(render_pass, 0);
    }

    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let resources: &mut SceneResource = resources.get_mut().unwrap();
        if let Some(request) = &self.camera_update {
            resources.update(queue, request);
        }
        Vec::new()
    }
}
