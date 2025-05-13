use eframe::egui_wgpu::{self, RenderState};
use fxhash::FxHashMap;
use wgpu::PrimitiveTopology;
use wgpu_shader::{
    camera::{Camera, ControlProperty, FollowCamera},
    constraint::{BindGroupImpl, PipelineConstraint},
    model,
    prelude::{
        glam::{Mat4, Vec3},
        Blend,
    },
    rgltf::{PlColor, PlNormal},
    types,
    uniform::UniformBuffer,
    util::GridDrawer,
    vertex::{VertexBuffer, VertexBufferSimple},
};

/// 基本のレンダリングリソースを作成する
pub fn init(rs: &RenderState, aspect: f32) {
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

    // グリッドの描画リソースを作成する
    if rs
        .renderer
        .read()
        .callback_resources
        .get::<RenderResource<PlColor>>()
        .is_none()
    {
        let mut rr = RenderResource::<PlColor>::new(
            &rs.device,
            rs.target_format,
            rs.renderer
                .read()
                .callback_resources
                .get::<SceneResource>()
                .unwrap(),
        );
        let grid = GridDrawer::default().gen_normal_color3(&rs.device);
        rr.primitives.insert(0, VertexWrap::Simple(grid));
        rr.add_material(
            &rs.device,
            "grid",
            UniformBuffer::new_encase(&rs.device, &types::uniform::Material::default()),
        );
        rr.add_draw_node(
            &rs.device,
            "grid",
            PipeType::LineList,
            0,
            "grid".to_string(),
            UniformBuffer::new_encase(&rs.device, &types::uniform::Model::from(&Mat4::IDENTITY)),
        );
        rs.renderer.write().callback_resources.insert(rr);
    }
}

/// サンプルシーンのリソースを設定する
pub fn sample(rs: &RenderState) {
    if let Some(rr) = rs
        .renderer
        .write()
        .callback_resources
        .get_mut::<RenderResource<PlColor>>()
    {
        let device = &rs.device;

        let vert = model::cube(1.0)
            .into_iter()
            .map(|x| types::vertex::NormalColor3 {
                position: Vec3::new(x.position.x, x.position.y, x.position.z),
                normal: Vec3::new(1.0, 0.0, 0.0),
                color: Vec3::new(x.color.x, x.color.y, x.color.z),
            })
            .collect::<Vec<_>>();
        let vb = VertexBufferSimple::new(device, &vert, None);
        rr.primitives.insert(1, VertexWrap::Simple(vb));

        let model =
            UniformBuffer::new_encase(device, &types::uniform::Model::from(&Mat4::IDENTITY));
        rr.add_draw_node(
            device,
            "cube",
            PipeType::Opacity,
            1,
            "default".to_string(),
            model,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeType {
    // トポロジがラインリスト
    LineList,
    // 不透明のトライアングル
    Opacity,
    // 透明のトライアングル
    Transparent,
}

/// 頂点データのラッパー
pub enum VertexWrap<V> {
    /// 頂点+インデックスバッファ
    Indexed(VertexBuffer<V>),
    /// 頂点バッファのみ
    Simple(VertexBufferSimple<V>),
}

impl<V> VertexWrap<V>
where
    V: bytemuck::Pod,
{
    fn set(&self, render_pass: &mut wgpu::RenderPass<'static>, slot: u32) {
        match self {
            VertexWrap::Indexed(vb) => vb.set(render_pass, slot),
            VertexWrap::Simple(vb) => vb.set(render_pass, slot),
        }
    }

    fn draw(&self, render_pass: &mut wgpu::RenderPass<'static>) {
        match self {
            VertexWrap::Indexed(vb) => render_pass.draw_indexed(0..vb.len(), 0, 0..1),
            VertexWrap::Simple(vb) => render_pass.draw(0..vb.len(), 0..1),
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
    pipelines: FxHashMap<PipeType, P>,
    // カメラのBindGroup
    cambgs: FxHashMap<u32, P::CameraBg>,
    // マテリアルのUniformBuffer
    materials: FxHashMap<String, UniformBuffer<P::Material>>,
    materials_bg: FxHashMap<String, P::MaterialBg>,
    // 描画するプリミティブの頂点データ
    primitives: FxHashMap<u32, VertexWrap<P::Vertex>>,
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
            (
                PipeType::LineList,
                PrimitiveTopology::LineList,
                Blend::Replace,
            ),
            (
                PipeType::Opacity,
                PrimitiveTopology::TriangleList,
                Blend::Replace,
            ),
            (
                PipeType::Transparent,
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
        pipetype: PipeType,
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
            pipetype,
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

    /// プリミティブの追加
    ///
    /// glTFの場合はプリミティブにマテリアル情報が連携されている
    pub fn add_primitive(
        &mut self,
        id: u32,
        primitive: VertexWrap<P::Vertex>,
        material_name: impl Into<String>,
    ) {
        self.primitives.insert(id, primitive);
        self.primitive_materials.insert(id, material_name.into());
    }

    pub fn get_material_name(&self, id: &u32) -> Option<&String> {
        self.primitive_materials.get(id)
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
                let pipe = self.pipelines.get(&obj.pipetype).unwrap();
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
    pipetype: PipeType,
}

#[derive(Debug, Clone)]
pub struct CameraUpdateRequest {
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
    /// カメラの更新
    pub camera_update: Option<CameraUpdateRequest>,
    /// レンダリングカメラセレクタ
    pub camera_id: u32,
}

impl RenderFrame {
    pub fn new() -> Self {
        Self {
            camera_update: None,
            camera_id: SceneResource::DEFAULT_CAMERA,
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
        // 使うレンダラの順序を決定
        if let Some(rr) = resources.get::<RenderResource<PlColor>>() {
            rr.paint(render_pass, self.camera_id);
        }
        if let Some(rr) = resources.get::<RenderResource<PlNormal>>() {
            rr.paint(render_pass, self.camera_id);
        }
    }

    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let rr: &mut SceneResource = resources.get_mut().unwrap();
        if let Some(request) = &self.camera_update {
            rr.update(queue, request);
        }
        Vec::new()
    }
}
