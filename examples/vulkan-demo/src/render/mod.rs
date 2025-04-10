use std::time::{Duration, Instant};

pub const BG_COLOR: wgpu::Color = wgpu::Color {
    r: 0.1,
    g: 0.2,
    b: 0.3,
    a: 1.0,
};

pub struct Timer {
    i: Instant,
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

impl Timer {
    pub fn new() -> Self {
        Self { i: Instant::now() }
    }

    pub fn ts(&self) -> Timestamp {
        let elapsed = self.i.elapsed();
        Timestamp {
            elapsed,
            delta: elapsed,
        }
    }
}

pub struct Timestamp {
    pub elapsed: Duration,
    pub delta: Duration,
}

pub mod particle {
    use wgpu_shader::{
        particle::*, uniform::UniformBuffer, vertex::VertexBufferSimple, WgpuContext,
    };

    use super::Timestamp;

    #[allow(dead_code)]
    pub struct Context {
        pipe: Pipeline,
        uniform: UniformBuffer<shader::Window>,
        vertexies: Vec<shader::VertexInput>,
        vb: VertexBufferSimple<shader::VertexInput>,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let u_w = shader::Window {
                resolution: [800.0, 600.0].into(),
                pixel_size: [12.0, 12.0].into(),
            };
            let uniform = UniformBuffer::new(state.device(), u_w);
            let mut pipe = Pipeline::new(state.device(), config, &uniform);
            pipe.set_bg_color(super::BG_COLOR);

            // init vertex
            let mut verts = vec![];
            for x in 0..10 {
                for y in 0..10 {
                    verts.push(shader::VertexInput {
                        position: [x as f32 * 0.1 - 0.5, y as f32 * 0.1 - 0.5, 0.0].into(),
                        color: [1.0, 0.0, 0.0].into(),
                    });
                }
            }

            let vb = VertexBufferSimple::new(state.device(), &verts, Some("Vertex Buffer"));

            Self {
                pipe,
                uniform,
                vertexies: verts,
                vb,
            }
        }

        pub fn input(&mut self, _event: &winit::event::WindowEvent) -> bool {
            false
        }

        pub fn update(&mut self, _state: &impl WgpuContext, _ts: &Timestamp) {}

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            self.pipe.render(state, &self.vb)
        }
    }
}

pub mod tutorial {

    use std::path::Path;

    use glam::Vec3;
    use nalgebra::{Rotation3, Scale3, Translation3, Vector3};
    use wgpu_shader::{
        prelude::*,
        tutorial::{shader::VertexInput, *},
        uniform::UniformBuffer,
        util::render,
        vertex::{InstanceBuffer, VertexBuffer},
        WgpuContext,
    };

    use crate::{
        camera::{Camera, CameraController, Cams},
        resources::ModelData,
    };

    use super::BG_COLOR;

    type Instance = shader::InstanceInput;

    // ライトの位置の時間変化
    const LIGHT_PASS: &[Vec3; 2] = &[glam::Vec3::new(2.0, 2.0, 2.0), Vec3::new(-2.0, 1.0, -2.0)];

    fn instances(len: i32, scale: f32, step: f32, translate: Vector3<f32>) -> Vec<Instance> {
        let mut instances = vec![];
        for z in 0..len {
            for x in 0..len {
                let pos = Vector3::new(x as f32 * step, 0.0, z as f32 * step) + translate;
                let rot = Rotation3::from_euler_angles(
                    (x as f32 * 0.1).to_degrees(),
                    (z as f32 * 0.1).to_degrees(),
                    0.0,
                );
                let scale = Scale3::new(scale, scale, scale);
                let mat = glam::Mat4::from(
                    Translation3::from(pos).to_homogeneous()
                        * rot.to_homogeneous()
                        * scale.to_homogeneous(),
                );
                let normal = glam::Mat4::from(rot.to_homogeneous());
                instances.push(Instance::from((mat, normal)));
            }
        }
        instances
    }

    pub struct Mesh {
        pub name: String,
        pub material: usize,
        pub vb: VertexBuffer<shader::VertexInput>,
    }

    pub struct Material {
        pub name: String,
        pub diffuse: TextureInst,
        pub bg: shader::bind_groups::BindGroup0,
    }

    pub struct Model {
        mesh: Mesh,
        material: Material,
    }

    impl Model {
        pub fn set(&self, rp: &mut wgpu::RenderPass<'_>) {
            self.material.bg.set(rp);
        }

        pub fn index_len(&self) -> u32 {
            self.mesh.vb.len()
        }
    }

    #[allow(dead_code)]
    pub struct Context {
        pipe_render: Pipeline,
        pipe_light: LightRenderPipeline,
        cam: Cams,
        cc: CameraController,
        ub_light: UniformBuffer<shader::Light>,
        model: Model,
        ib: InstanceBuffer<Instance>,
    }

    impl Context {
        pub fn new(
            state: &impl WgpuContext,
            config: &wgpu::SurfaceConfiguration,
            assets_dir: &Path,
        ) -> Self {
            let cam = Camera::with_aspect(config.width as f32 / config.height as f32);
            let cam = Cams::new(state.device(), cam);
            let cc = CameraController::new(0.01);
            let ub_light = UniformBuffer::new(state.device(), create_light());

            let pipe_render = Pipeline::new(state.device(), config, cam.buffer(), &ub_light);

            let model = Self::load_model(state, &assets_dir.join("models/cube/cube.obj"));
            let pipe_light =
                LightRenderPipeline::new(state.device(), config, cam.buffer(), &ub_light);
            let ib = InstanceBuffer::new(
                state.device(),
                &instances(10, 0.1, 0.3, Vector3::new(-2.0, 0.0, -2.0)),
            );

            Self {
                pipe_render,
                pipe_light,
                cam,
                cc,
                ub_light,
                model,
                ib,
            }
        }

        /// モデルデータの読み込み。wgpuにはまだ載せない
        fn load_model(state: &impl WgpuContext, path: &Path) -> Model {
            let model_data = ModelData::from_path(path).expect("Failed to load model data");
            let m = model_data.models.first().expect("Model data is empty");
            let mesh = Self::load_model_inner(state, m);
            let mat = model_data
                .materials
                .first()
                .expect("Material data is empty");
            let tex = load_texture(state, &model_data.texture_path(mat).unwrap())
                .expect("Failed to load texture");
            let bg0 = shader::bind_groups::BindGroup0::from_bindings(state.device(), tex.desc());
            let material = Material {
                name: mat.name.clone(),
                diffuse: tex,
                bg: bg0,
            };
            Model { mesh, material }
        }

        fn load_model_inner(state: &impl WgpuContext, m: &tobj::Model) -> Mesh {
            let vertices = 0..m.mesh.positions.len() / 3;

            let vertices = vertices
                .map(|i| VertexInput {
                    position: [
                        m.mesh.positions[i * 3],
                        m.mesh.positions[i * 3 + 1],
                        m.mesh.positions[i * 3 + 2],
                    ]
                    .into(),
                    tex_coords: [m.mesh.texcoords[i * 2], m.mesh.texcoords[i * 2 + 1]].into(),
                    normal: [
                        m.mesh.normals[i * 3],
                        m.mesh.normals[i * 3 + 1],
                        m.mesh.normals[i * 3 + 2],
                    ]
                    .into(),
                })
                .collect::<Vec<_>>();
            let indexies = m.mesh.indices.iter().map(|x| *x as u16).collect::<Vec<_>>();
            let vb = VertexBuffer::new(state.device(), &vertices, &indexies);

            Mesh {
                name: m.name.clone(),
                material: m.mesh.material_id.unwrap_or(0),
                vb,
            }
        }

        pub fn input(&mut self, event: &winit::event::WindowEvent) -> bool {
            self.cc.process_events(event)
        }

        pub fn resize(&mut self, state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) {
            self.cam
                .camera_mut()
                .camera_mut()
                .set_aspect(config.width as f32 / config.height as f32);
            self.cam.update(state.queue());
        }

        pub fn update(&mut self, state: &impl WgpuContext, ts: &super::Timestamp) {
            self.cc.update_follow_camera(self.cam.camera_mut());
            self.cam.update(state.queue());

            // 時間でライトの位置を変化させる
            let w = ts.elapsed.as_secs_f32().sin();
            let pos = LIGHT_PASS[0] * w + LIGHT_PASS[1] * (1.0 - w);
            let light: shader::Light = shader::Light {
                position: pos,
                color: Vec3::new(1.0, 1.0, 1.0),
            };
            self.ub_light.write(state.queue(), &light);
        }

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            render(state, BG_COLOR, state.depth(), |rp| {
                let r = &self.pipe_light;
                let vb = &self.model.mesh.vb;
                r.set(rp, vb);
                rp.draw_indexed(0..vb.len(), 0, 0..1);

                let r = &self.pipe_render;
                r.set(rp, vb, &self.ib);
                self.model.set(rp);
                rp.draw_indexed(0..vb.len(), 0, 0..self.ib.len());
            })
        }
    }

    // テクスチャ読み込み
    fn load_texture(state: &impl WgpuContext, path: &Path) -> Result<TextureInst, std::io::Error> {
        let img = std::fs::read(path)?;
        let img = image::load_from_memory(&img).unwrap();
        let img = img.to_rgba8();
        let dimensions = img.dimensions();

        let texture_size = wgpu::Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 1,
        };

        let tex = state.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let inst = TextureInst::new(state.device(), tex);
        inst.write(state.queue(), &img, dimensions, texture_size);

        Ok(inst)
    }
}

pub mod unif {

    use std::{collections::VecDeque, time::Duration};

    use glam::{Mat4, Vec3};
    use wgpu_shader::{
        colored, graph, model,
        prelude::*,
        types,
        uniform::UniformBuffer,
        util::{render, GridDrawer},
        vertex::{Topology, VbBinding, VertexBufferSimple},
        WgpuContext,
    };

    use crate::camera::{Camera, CameraController, Cams, FollowCamera};

    use super::BG_COLOR;

    type ModelNode = graph::ModelNode<SlotType>;
    type ModelGraph = graph::ModelGraph<ModelNode>;

    enum SlotType {
        // 表示内容を含むオブジェクト
        Draw(Drawable),
        // 表示位置を調整するための座標点としてのオブジェクト
        Bone,
        // 親を視点とするカメラ
        FollowCamera(CameraObj),
        // 影を描画するためのオブジェクト
        Shadow(FloorShadow),
        Transparent(Drawable),
    }

    impl ModelNodeImplClone for SlotType {
        fn clone_object(&self, device: &wgpu::Device) -> Self {
            match self {
                SlotType::Draw(d) => SlotType::Draw(d.clone_object(device)),
                SlotType::Bone => SlotType::Bone,
                SlotType::FollowCamera(c) => SlotType::FollowCamera(c.clone_object(device)),
                SlotType::Shadow(s) => SlotType::Shadow(s.clone_object(device)),
                SlotType::Transparent(t) => SlotType::Transparent(t.clone_object(device)),
            }
        }
    }

    impl PartialEq for SlotType {
        fn eq(&self, other: &Self) -> bool {
            matches!(
                (self, other),
                (SlotType::Draw(_), SlotType::Draw(_))
                    | (SlotType::Bone, SlotType::Bone)
                    | (SlotType::FollowCamera(_), SlotType::FollowCamera(_))
                    | (SlotType::Shadow(_), SlotType::Shadow(_))
                    | (SlotType::Transparent(_), SlotType::Transparent(_))
            )
        }
    }

    impl PartialOrd for SlotType {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            match (self, other) {
                (SlotType::Draw(_), SlotType::Shadow(_)) => Some(std::cmp::Ordering::Less),
                (SlotType::Draw(_), SlotType::Transparent(_)) => Some(std::cmp::Ordering::Less),
                (SlotType::Shadow(_), SlotType::Transparent(_)) => Some(std::cmp::Ordering::Less),
                _ => Some(std::cmp::Ordering::Equal),
            }
        }
    }

    impl From<Drawable> for SlotType {
        fn from(d: Drawable) -> Self {
            Self::Draw(d)
        }
    }

    impl From<FloorShadow> for SlotType {
        fn from(d: FloorShadow) -> Self {
            Self::Shadow(d)
        }
    }

    struct Drawable {
        color: glam::Vec4,
        buffer: UniformBuffer<colored::unif::DrawInfo>,
        bg: colored::DrawInfoBindGroup,
        vb: VbBinding,
    }

    impl Drawable {
        fn new(device: &wgpu::Device, color: glam::Vec4, vb: VbBinding) -> Self {
            let buffer = colored::unif::DrawInfo {
                matrix: glam::Mat4::IDENTITY,
                color,
            };
            let buffer = UniformBuffer::new(device, buffer);
            let bg = colored::PlUnif::make_draw_unif(device, &buffer);
            Self {
                color,
                buffer,
                bg,
                vb,
            }
        }

        fn color(&mut self, color: glam::Vec4) {
            self.color = color;
        }

        fn update(&mut self, queue: &wgpu::Queue, matrix: Mat4) {
            let buffer = colored::unif::DrawInfo {
                matrix,
                color: self.color,
            };
            self.buffer.write(queue, &buffer);
        }

        fn draw(&self, rp: &mut wgpu::RenderPass<'_>) {
            self.bg.set(rp);
            self.vb.set(rp);
            rp.draw(0..self.vb.len(), 0..1);
        }

        // オブジェクトの複製
        fn clone_object(&self, device: &wgpu::Device) -> Self {
            let buffer = self.buffer.clone_object(device);
            let bg = colored::PlUnif::make_draw_unif(device, &buffer);
            let vb = self.vb.clone();
            Self {
                color: self.color,
                buffer,
                bg,
                vb,
            }
        }
    }

    struct FloorShadow {
        color: glam::Vec4,
        scale: glam::Vec3,
        buffer: UniformBuffer<colored::unif::DrawInfo>,
        bg: colored::DrawInfoBindGroup,
        vb: VbBinding,
    }

    impl FloorShadow {
        const SHADOW_COLLOR: glam::Vec4 = glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
        fn new(device: &wgpu::Device, vb: VbBinding) -> Self {
            let buffer = colored::unif::DrawInfo {
                matrix: glam::Mat4::IDENTITY,
                color: Self::SHADOW_COLLOR,
            };
            let buffer = UniformBuffer::new(device, buffer);
            let bg = colored::PlUnif::make_draw_unif(device, &buffer);
            Self {
                color: Self::SHADOW_COLLOR,
                buffer,
                bg,
                vb,
                scale: glam::Vec3::new(1.0, 1.0, 0.0),
            }
        }

        fn update(&mut self, queue: &wgpu::Queue, matrix: Mat4) {
            let scale = glam::Mat4::from_scale(self.scale);
            let matrix = scale * matrix;
            let buffer = colored::unif::DrawInfo {
                matrix,
                color: self.color,
            };
            self.buffer.write(queue, &buffer);
        }

        fn draw(&self, rp: &mut wgpu::RenderPass<'_>) {
            self.bg.set(rp);
            self.vb.set(rp);
            rp.draw(0..self.vb.len(), 0..1);
        }

        fn clone_object(&self, device: &wgpu::Device) -> Self {
            Self::new(device, self.vb.clone())
        }
    }

    struct CameraObj {
        cam: Cams,
    }

    impl CameraObj {
        fn new(cam: Cams) -> Self {
            Self { cam }
        }

        fn get_cam_mut(&mut self) -> &mut FollowCamera {
            self.cam.camera_mut()
        }

        fn update(&mut self, queue: &wgpu::Queue, matrix: Mat4) {
            self.cam.update_world_pos(queue, matrix);
        }

        fn clone_object(&self, debice: &wgpu::Device) -> Self {
            Self::new(self.cam.clone_object(debice))
        }
    }

    impl From<Cams> for CameraObj {
        fn from(cam: Cams) -> Self {
            Self::new(cam)
        }
    }

    // 任意の時点のオブジェクトの履歴を保持する。
    // 軌跡表示のみで
    struct ObjectHistory<N> {
        v: VecDeque<N>,
        limit_len: usize,
        latest: Duration,
        interval: Duration,
        node: String,
    }

    impl<N> ObjectHistory<N>
    where
        N: ModelNodeImplClone,
    {
        fn new(node: String) -> Self {
            Self {
                v: VecDeque::new(),
                limit_len: 60 * 10,
                latest: Duration::new(0, 0),
                interval: Duration::from_millis(100),
                node,
            }
        }

        fn update(
            &mut self,
            state: &impl WgpuContext,
            graph: &graph::ModelGraph<N>,
            ts: &super::Timestamp,
        ) {
            if self.latest + self.interval < ts.elapsed {
                // 100msごとに履歴を保存
                self.latest = ts.elapsed;
                if let Some(b) = graph.get_node(&self.node) {
                    // もし履歴がいっぱいなら古いものを削除
                    if self.v.len() == self.limit_len {
                        self.v.pop_front();
                    }
                    // 履歴に追加
                    self.v.push_back(b.clone_object(state.device()));
                }
            }
        }

        fn iter(&self) -> impl Iterator<Item = &N> {
            self.v.iter()
        }
    }

    pub struct Context {
        p_poly: colored::PlUnif,
        p_poly_trans: colored::PlUnif,
        p_line: colored::PlUnif,
        cc: CameraController,
        // bufferは参照をノードに渡して使っており、直接参照しない
        _grid: VertexBufferSimple<types::vertex::Color4>,
        _vb: VertexBufferSimple<types::vertex::Color4>,
        _frustum_vb: VertexBufferSimple<types::vertex::Color4>,
        graph: ModelGraph,
        history: ObjectHistory<ModelNode>,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let cam = Camera::with_aspect(config.width as f32 / config.height as f32);
            let cam = Cams::new(state.device(), cam);

            let cc = CameraController::new(0.05);
            let p_poly = colored::PlUnif::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::TriangleList,
                Blend::Replace,
            );
            let p_line = colored::PlUnif::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::LineList,
                Blend::Replace,
            );
            let p_poly_trans = colored::PlUnif::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::TriangleList,
                Blend::Alpha,
            );
            let grid_vb = GridDrawer::default().gen(state.device());
            let vb = VertexBufferSimple::new(state.device(), &model::cube(1.0), None);
            let frustum_vb = VertexBufferSimple::new(
                state.device(),
                &model::frustum(model::FrustumParam::new(
                    20_f32.to_radians(),
                    1.33,
                    0.2,
                    6.0,
                )),
                Some("Camera frustum"),
            );
            let mut graph = Self::create_model(
                state.device(),
                grid_vb.bind_buffer(0, Topology::LineList),
                vb.bind_buffer(0, Topology::TriangleList),
            )
            .unwrap();
            graph
                .add_node(
                    Some("b2"),
                    ModelNode::new("cam", Trs::default(), SlotType::FollowCamera(cam.into())),
                )
                .unwrap();
            graph
                .add_node(
                    Some("b4"),
                    ModelNode::new(
                        "cv",
                        Trs::default(),
                        SlotType::Transparent(Drawable::new(
                            state.device(),
                            glam::Vec4::new(1.0, 1.0, 1.0, 0.2),
                            frustum_vb.bind_buffer(0, Topology::TriangleList),
                        )),
                    ),
                )
                .unwrap();
            graph
                .add_node(
                    Some("b4"),
                    ModelNode::new(
                        "cv-shadow",
                        Trs::default(),
                        SlotType::Shadow(FloorShadow::new(
                            state.device(),
                            frustum_vb.bind_buffer(0, Topology::TriangleList),
                        )),
                    ),
                )
                .unwrap();
            for node in graph.iter_mut() {
                let world = node.world();
                if let SlotType::Draw(ref mut obj) = node.value_mut() {
                    obj.update(state.queue(), world);
                }
            }
            Self {
                p_poly,
                p_line,
                p_poly_trans,
                cc,
                _grid: grid_vb,
                _vb: vb,
                _frustum_vb: frustum_vb,
                graph,
                history: ObjectHistory::new("d4".to_string()),
            }
        }

        pub fn input(&mut self, event: &winit::event::WindowEvent) -> bool {
            self.cc.process_events(event)
        }

        fn update_camera(&mut self, state: &impl WgpuContext) {
            let node = self.graph.get_must_mut("cam");

            let matrix = node.world();
            if let SlotType::FollowCamera(ref mut obj) = node.value_mut() {
                self.cc.update_follow_camera(obj.get_cam_mut());
                obj.update(state.queue(), matrix);
            }
        }

        pub fn update(&mut self, state: &impl WgpuContext, ts: &super::Timestamp) {
            self.update_camera(state);

            let s = ts.elapsed.as_secs_f32();

            // モデルの座標更新
            let graph = &mut self.graph;
            graph.get_must_mut("b1").trs_mut().set_rot_y(s);
            graph.get_must_mut("b2").trs_mut().set_rot_z(s * 1.2);
            graph.get_must_mut("b3").trs_mut().set_rot_x(s * 1.4);
            graph.update_world("b1").unwrap();

            // 更新対象のみuniformを更新
            for node in graph.iter_mut() {
                if node.get_updated() {
                    let world = node.world();
                    match node.value_mut() {
                        SlotType::Draw(ref mut obj) => {
                            obj.update(state.queue(), world);
                        }
                        SlotType::Shadow(ref mut obj) => {
                            obj.update(state.queue(), world);
                        }
                        SlotType::Transparent(ref mut obj) => {
                            obj.update(state.queue(), world);
                        }
                        _ => {}
                    }
                }
            }

            self.history.update(state, graph, ts);
        }

        /// レンダリング
        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            render(state, BG_COLOR, state.depth(), |rp| {
                for hist in self.history.iter() {
                    if let SlotType::Draw(ref obj) = hist.value() {
                        match obj.vb.topology() {
                            Topology::TriangleList => self.p_poly.set(rp),
                            Topology::LineList => self.p_line.set(rp),
                        }
                        obj.draw(rp);
                    }
                }
                let mut objs = self.graph.iter().collect::<Vec<_>>();
                objs.sort_by(|a, b| a.value().partial_cmp(b.value()).unwrap());
                for obj in objs.into_iter() {
                    match obj.value() {
                        SlotType::Draw(obj) => {
                            match obj.vb.topology() {
                                Topology::TriangleList => self.p_poly.set(rp),
                                Topology::LineList => self.p_line.set(rp),
                            }
                            obj.draw(rp);
                        }
                        SlotType::Shadow(obj) => {
                            match obj.vb.topology() {
                                Topology::TriangleList => self.p_poly.set(rp),
                                Topology::LineList => self.p_line.set(rp),
                            }
                            obj.draw(rp);
                        }
                        SlotType::Transparent(obj) => {
                            match obj.vb.topology() {
                                Topology::TriangleList => self.p_poly_trans.set(rp),
                                Topology::LineList => self.p_line.set(rp),
                            }
                            obj.draw(rp);
                        }
                        _ => {}
                    }
                }
            })
        }

        fn create_model(
            device: &wgpu::Device,
            floor_vb: VbBinding,
            cube_vb: VbBinding,
        ) -> anyhow::Result<ModelGraph> {
            let l = [
                (None, "floor", Trs::default(), SlotType::Bone),
                (
                    Some("floor"),
                    "floor-d1",
                    Trs::default(),
                    Drawable::new(device, glam::Vec4::ONE, floor_vb.clone()).into(),
                ),
                (None, "b1", Trs::default(), SlotType::Bone),
                (
                    Some("b1"),
                    "d1",
                    Trs::with_s(0.1),
                    Drawable::new(device, glam::Vec4::ONE, cube_vb.clone()).into(),
                ),
                (Some("b1"), "b2", Trs::with_t(Vec3::Z * 0.5), SlotType::Bone),
                (
                    Some("b2"),
                    "d2",
                    Trs::with_s(0.1),
                    Drawable::new(device, glam::Vec4::ONE, cube_vb.clone()).into(),
                ),
                (
                    Some("b2"),
                    "ds2",
                    Trs::with_s(0.1),
                    FloorShadow::new(device, cube_vb.clone()).into(),
                ),
                (Some("b2"), "b3", Trs::with_t(Vec3::Z * 0.5), SlotType::Bone),
                (
                    Some("b3"),
                    "d3",
                    Trs::with_s(0.1),
                    Drawable::new(device, glam::Vec4::ONE, cube_vb.clone()).into(),
                ),
                (
                    Some("b3"),
                    "ds3",
                    Trs::with_s(0.1),
                    FloorShadow::new(device, cube_vb.clone()).into(),
                ),
                (Some("b3"), "b4", Trs::with_t(Vec3::Z * 0.5), SlotType::Bone),
                (
                    Some("b4"),
                    "d4",
                    Trs::with_s(0.1),
                    Drawable::new(device, glam::Vec4::ONE, cube_vb.clone()).into(),
                ),
                (
                    Some("b4"),
                    "ds4",
                    Trs::with_s(0.1),
                    FloorShadow::new(device, cube_vb.clone()).into(),
                ),
            ];

            let mut graph = ModelGraph::default();

            for (parent, name, trs, slot) in l.into_iter() {
                let model = ModelNode::new(name, trs, slot);
                graph.add_node(parent, model)?;
            }

            Ok(graph)
        }
    }
}

/// 依存関係のあるオブジェクト位置変換に使う
pub struct MatrixStack {
    matrix: glam::Mat4,
    stack: Vec<glam::Mat4>,
}

impl Default for MatrixStack {
    fn default() -> Self {
        Self::new()
    }
}

impl MatrixStack {
    pub fn new() -> Self {
        Self {
            matrix: glam::Mat4::IDENTITY,
            stack: vec![],
        }
    }

    /// 現在の行列をスタックに保存
    pub fn save(&mut self) {
        self.stack.push(self.matrix);
    }

    pub fn restore(&mut self) {
        self.matrix = self.stack.pop().expect("Matrix stack is empty");
    }

    pub fn get(&self) -> glam::Mat4 {
        self.matrix
    }

    pub fn set(&mut self, mat: glam::Mat4) {
        self.matrix = mat;
    }

    pub fn dot(&mut self, mat: glam::Mat4) {
        self.matrix *= mat
    }

    pub fn translate(&mut self, vec: glam::Vec3) {
        self.matrix = glam::Mat4::from_translation(vec) * self.matrix;
    }

    pub fn rotate_x(&mut self, rad: f32) {
        self.matrix = glam::Mat4::from_rotation_x(rad) * self.matrix;
    }

    pub fn rotate_y(&mut self, rad: f32) {
        self.matrix = glam::Mat4::from_rotation_y(rad) * self.matrix;
    }

    pub fn rotate_z(&mut self, rad: f32) {
        self.matrix = glam::Mat4::from_rotation_z(rad) * self.matrix;
    }

    pub fn scale(&mut self, vec: glam::Vec3) {
        self.matrix = glam::Mat4::from_scale(vec) * self.matrix;
    }
}
