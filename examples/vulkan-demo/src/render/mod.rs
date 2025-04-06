use std::time::{Duration, Instant};

mod scene;

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

pub mod colored {

    use nalgebra::{UnitQuaternion, Vector3};

    use wgpu_shader::{
        colored::{compress, PipelineComp, PipelineInstanced, PipelinePrim},
        model::{cube, hand4, rect, CUBE_INDEX, RECT_INDEX},
        types,
        uniform::UniformBuffer,
        util::{render, GridDrawer},
        vertex::{InstanceBuffer, VertexBuffer, VertexBufferSimple},
        WgpuContext,
    };

    use crate::camera::{Camera, CameraController, Cams, ROTATION_FACE_Z_TO_X};

    use super::BG_COLOR;

    type HandBuffer = VertexBufferSimple<types::vertex::Color4>;

    pub struct ObjectPrim {
        // 位置
        pub pos: Vector3<f32>,
        pub target: Vector3<f32>,
        // 上方向
        pub up: Vector3<f32>,
    }

    impl ObjectPrim {
        pub fn new(pos: Vector3<f32>, target: Vector3<f32>, up: Vector3<f32>) -> Self {
            Self { pos, target, up }
        }

        pub fn isometry(&self) -> glam::Mat4 {
            let t = nalgebra::Translation3::from(self.pos);
            let r = nalgebra::Rotation3::look_at_rh(&(self.target - self.pos), &self.up).inverse();
            glam::Mat4::from(t.to_homogeneous() * r.to_homogeneous() * ROTATION_FACE_Z_TO_X)
        }

        /// レンダリング時に与えられるオフセットを考慮して位置補正行列を返す
        pub fn offset_isometry(&self, offset: Vector3<f32>) -> glam::Mat4 {
            let pos = self.pos + offset;
            let t = nalgebra::Translation3::from(pos);
            // ターゲットの方向を向かせるため、ターゲット向けのベクトルを作成、look_at_rhしてその逆行列を取る
            let r = nalgebra::Rotation3::look_at_rh(&(pos - self.target), &self.up).inverse();
            glam::Mat4::from(t.to_homogeneous() * r.to_homogeneous() * ROTATION_FACE_Z_TO_X)
        }
    }

    /// カメラから見えているものを投影するテスト
    struct Recam {
        x: f32,
        z: f32,
        q: UnitQuaternion<f32>,
        pos: Vector3<f32>,
    }

    impl Recam {
        fn new(x: f32, z: f32, pos: Vector3<f32>) -> Self {
            let q = Self::quatenion(x, z);
            Self { x, z, q, pos }
        }

        fn update(&mut self) {
            self.q = Self::quatenion(self.x, self.z);
        }

        fn quatenion(x: f32, z: f32) -> UnitQuaternion<f32> {
            UnitQuaternion::from_euler_angles(0.0, x, z)
        }

        fn translate(&self) -> glam::Mat4 {
            let t = nalgebra::Translation3::from(self.pos);
            glam::Mat4::from(t.to_homogeneous())
        }

        fn isometry(&self) -> glam::Mat4 {
            let t = self.translate();
            let r = glam::Mat4::from(self.q.inverse().to_homogeneous());
            t * r
        }
    }

    struct Recams {
        // カメラとその軸表示
        recam: Recam,
        recam_inst: InstanceBuffer<types::instance::Isometry>,
        // カメラから見えている物体とその表示
        object: ObjectPrim,
        object_inst: InstanceBuffer<types::instance::Isometry>,
    }

    impl Recams {
        fn new(device: &wgpu::Device) -> Self {
            let recam = Recam::new(30_f32.to_radians(), 0.0, Vector3::new(0.0, 0.0, 0.4));
            let mat = recam.isometry();
            let recam_inst = InstanceBuffer::new(device, &[types::instance::Isometry::new(mat)]);
            let object = ObjectPrim::new(
                Vector3::new(3.0, 0.0, 0.0),
                Vector3::new(3.0, 3.0, 0.0),
                Vector3::z(),
            );
            // カメラの行列で変換をかけることカメラ視野 -> ワールド座標になる
            let mat = mat * object.isometry();
            let object_inst = InstanceBuffer::new(device, &[types::instance::Isometry::new(mat)]);
            Self {
                recam,
                recam_inst,
                object,
                object_inst,
            }
        }

        fn update(&mut self, queue: &wgpu::Queue) {
            self.recam.update();
            let mat = self.recam.isometry();
            let inst = types::instance::Isometry::new(mat);
            self.recam_inst.update(queue, &[inst]);
            let mat = mat * self.object.isometry();
            let inst = types::instance::Isometry::new(mat);
            self.object_inst.update(queue, &[inst]);
        }

        fn draw_guide(&self, rp: &mut wgpu::RenderPass<'_>, hand: &HandBuffer) {
            self.recam_inst.set(rp, 1);
            rp.draw(0..hand.len(), 0..self.recam_inst.len());
        }

        fn draw_cube(
            &self,
            rp: &mut wgpu::RenderPass<'_>,
            cube_vb: &VertexBuffer<types::vertex::Color4>,
        ) {
            self.object_inst.set(rp, 1);
            rp.draw_indexed(0..cube_vb.len(), 0, 0..self.recam_inst.len());
        }

        fn draw_shadow(
            &self,
            rp: &mut wgpu::RenderPass<'_>,
            rect_vb: &VertexBuffer<types::vertex::Color4>,
        ) {
            self.object_inst.set(rp, 1);
            rp.draw_indexed(0..rect_vb.len(), 0, 0..self.recam_inst.len());
        }
    }

    pub struct Context {
        p0: PipelinePrim,
        p1_line: PipelineInstanced,
        p1_plane: PipelineInstanced,
        p2_plane: PipelineComp,
        cam: Cams,
        cc: CameraController,
        _ub_comp: UniformBuffer<compress::Compression>,
        vb: VertexBufferSimple<types::vertex::Color4>,
        hand: HandBuffer,
        hands: Vec<ObjectPrim>,
        hi: InstanceBuffer<types::instance::Isometry>,
        cube_vb: VertexBuffer<types::vertex::Color4>,
        recams: Recams,
        rect_vb: VertexBuffer<types::vertex::Color4>,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let cam = Camera::with_aspect(config.width as f32 / config.height as f32);
            let cam = Cams::new(state.device(), cam);
            let cc = CameraController::new(0.05);

            let p0 = PipelinePrim::new(state.device(), config, cam.buffer());
            let p1 = PipelineInstanced::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::LineList,
            );
            let p1_plane = PipelineInstanced::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::TriangleList,
            );

            let c = compress::Compression::xy();
            let ub_comp = UniformBuffer::new(state.device(), c);
            let p2_plane = PipelineComp::new(
                state.device(),
                config,
                cam.buffer(),
                &ub_comp,
                wgpu::PrimitiveTopology::TriangleList,
            );
            let vb = GridDrawer::default().gen(state.device());
            let hand = hand_arrow(state.device());
            let hands = vec![
                ObjectPrim::new(
                    Vector3::new(1.0, 0.0, 0.0),
                    Vector3::new(0.0, 0.0, 0.0),
                    Vector3::z(),
                ),
                ObjectPrim::new(
                    Vector3::new(0.0, 1.0, 0.5),
                    Vector3::new(0.0, 0.0, 0.0),
                    Vector3::z(),
                ),
            ];
            let hi = hands_instance(state.device(), &hands);
            let cube_vb = VertexBuffer::new(state.device(), &cube(0.2), &CUBE_INDEX);
            let rect_vb = VertexBuffer::new(state.device(), &rect(0.2), &RECT_INDEX);

            let recams = Recams::new(state.device());

            Self {
                p0,
                p1_line: p1,
                p1_plane,
                p2_plane,
                cam,
                cc,
                _ub_comp: ub_comp,
                vb,
                hand,
                hands,
                hi,
                cube_vb,
                recams,
                rect_vb,
            }
        }

        /// レンダリング
        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            render(state, BG_COLOR, state.depth(), |rp| {
                self.p0.set(rp);
                // 頂点バッファをセットして描画
                self.vb.set(rp, 0);
                rp.draw(0..self.vb.len(), 0..1);

                self.p1_line.set(rp);
                self.hand.set(rp, 0);
                self.hi.set(rp, 1);
                rp.draw(0..self.hand.len(), 0..self.hi.len());
                self.recams.draw_guide(rp, &self.hand);

                self.p1_plane.set(rp);
                self.cube_vb.set(rp, 0);
                self.hi.set(rp, 1);
                rp.draw_indexed(0..self.cube_vb.len(), 0, 0..self.hi.len());
                self.recams.draw_cube(rp, &self.cube_vb);

                self.p2_plane.set(rp);
                self.rect_vb.set(rp, 0);
                self.hi.set(rp, 1);
                rp.draw_indexed(0..self.rect_vb.len(), 0, 0..self.hi.len());
                self.recams.draw_shadow(rp, &self.rect_vb);
            })
        }

        pub fn input(&mut self, event: &winit::event::WindowEvent) -> bool {
            self.cc.process_events(event)
        }

        pub fn update(&mut self, state: &impl WgpuContext, ts: &super::Timestamp) {
            self.cc.update_follow_camera(self.cam.camera_mut());
            self.cam.update(state.queue());

            // 時間経過でXY平面方向に移動
            let s = ts.elapsed.as_secs_f32();
            let offset = Vector3::new(s.sin() * 2.0, s.cos(), 0.0);
            let hands = self
                .hands
                .iter()
                .map(|h| types::instance::Isometry::new(h.offset_isometry(offset)))
                .collect::<Vec<_>>();
            self.hi.update(state.queue(), &hands);

            // カメラの位置と向きを変える
            self.recams.recam.pos = Vector3::new(s.cos() * 0.5, 0.0, 0.2);
            self.recams.recam.z = (s * 1.5).sin() * 30_f32.to_radians();
            self.recams.update(state.queue());
        }
    }

    /// 3D空間での矢印を描画する。原点からXYZ方向にそれぞれ0.3mの長さを持つ
    pub fn hand_arrow(device: &wgpu::Device) -> HandBuffer {
        VertexBufferSimple::new(device, &hand4(0.3), Some("Hand Arrow"))
    }

    pub fn hands_instance(
        device: &wgpu::Device,
        hands: &[ObjectPrim],
    ) -> InstanceBuffer<types::instance::Isometry> {
        let mut instances = vec![];
        for h in hands.iter() {
            instances.push(types::instance::Isometry::new(h.isometry()));
        }
        InstanceBuffer::new(device, &instances)
    }
}

pub mod unif {

    use glam::Vec3;
    use wgpu_shader::{
        colored::{self, unif, GlobalUnif},
        model, types,
        uniform::UniformBuffer,
        util::render,
        vertex::VertexBufferSimple,
        WgpuContext,
    };

    use crate::camera::{Camera, CameraController, Cams};

    use super::{MatrixStack, BG_COLOR};

    struct DrawObject {
        // 配置位置変更
        translate: glam::Vec3,
        // 最後のスケール
        scale: glam::Vec3,
        color: glam::Vec4,
        buffer: UniformBuffer<colored::unif::DrawInfo>,
        bg: colored::DrawInfoBindGroup,
    }

    #[allow(unused)]
    impl DrawObject {
        fn new(
            device: &wgpu::Device,
            translate: glam::Vec3,
            scale: glam::Vec3,
            color: glam::Vec4,
        ) -> Self {
            let mat = glam::Mat4::from_translation(translate) * glam::Mat4::from_scale(scale);
            let buffer = colored::unif::DrawInfo { matrix: mat, color };
            let buffer = UniformBuffer::new(device, buffer);
            let bg = colored::PlUnif::make_draw_unif(device, &buffer);
            Self {
                translate,
                scale,
                color,
                buffer,
                bg,
            }
        }

        fn set_trans(&mut self, translate: glam::Vec3) {
            self.translate = translate;
        }

        fn set_scale(&mut self, scale: glam::Vec3) {
            self.scale = scale;
        }

        fn set_color(&mut self, color: glam::Vec4) {
            self.color = color;
        }

        fn update(&mut self, queue: &wgpu::Queue, ctx: &MatrixStack) {
            // ローカルTranslate変換 -> 外部からの変形適用 -> 最終スケール合わせ
            let mat = glam::Mat4::from_translation(self.translate);
            let buffer = colored::unif::DrawInfo {
                matrix: mat * ctx.get() * glam::Mat4::from_scale(self.scale),
                color: self.color,
            };
            self.buffer.write(queue, &buffer);
        }

        fn draw(&self, rp: &mut wgpu::RenderPass<'_>) {
            self.bg.set(rp);
        }
    }

    struct Objects {
        global: GlobalUnif,
        objs: Vec<DrawObject>,
    }
    impl Objects {
        fn new(g2l: GlobalUnif, objs: Vec<DrawObject>) -> Self {
            Self { global: g2l, objs }
        }
    }

    pub struct Context {
        p0: colored::PlUnif,
        cam: Cams,
        cc: CameraController,
        vb: VertexBufferSimple<types::vertex::Color4>,
        o: Objects,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let cam = Camera::with_aspect(config.width as f32 / config.height as f32);
            let cam = Cams::new(state.device(), cam);
            let cc = CameraController::new(0.05);
            let p0 = colored::PlUnif::new(
                state.device(),
                config,
                cam.buffer(),
                wgpu::PrimitiveTopology::TriangleList,
            );
            let vb = VertexBufferSimple::new(state.device(), &model::cube(1.0), None);
            let objs = Self::create_object(state.device());
            let g2l = GlobalUnif::new(state.device());
            Self {
                p0,
                cam,
                cc,
                vb,
                o: Objects::new(g2l, objs),
            }
        }

        pub fn input(&mut self, event: &winit::event::WindowEvent) -> bool {
            self.cc.process_events(event)
        }

        pub fn update(&mut self, state: &impl WgpuContext, ts: &super::Timestamp) {
            self.cc.update_follow_camera(self.cam.camera_mut());
            self.cam.update(state.queue());

            let s = ts.elapsed.as_secs_f32();

            // オブジェクトのグローバル座標を調整
            let mut ms = MatrixStack::new();
            ms.translate(Vec3::new(0.0, 0.0, s.sin()));
            ms.rotate_z(s * 0.1);
            let scale = s.cos() * 0.25 + 1.0;
            ms.scale(Vec3::new(scale, scale, scale));
            self.o
                .global
                .write(state.queue(), &unif::GlobalInfo { matrix: ms.get() });

            // あとのobjectのほうがより回転がきつくなるような動作
            let mut ms = MatrixStack::new();
            for obj in self.o.objs.iter_mut() {
                ms.rotate_y((s * 0.1).sin());
                obj.update(state.queue(), &ms);
            }
        }

        /// レンダリング
        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            render(state, BG_COLOR, state.depth(), |rp| {
                self.p0.set(rp);
                // 頂点バッファをセットして描画
                self.vb.set(rp, 0);

                self.o.global.set(rp);
                for obj in self.o.objs.iter() {
                    obj.draw(rp);
                    rp.draw(0..self.vb.len(), 0..1);
                }
            })
        }

        fn create_object(device: &wgpu::Device) -> Vec<DrawObject> {
            let len = 10;
            let trans_rate = 0.4;
            let offset = trans_rate * (len as f32 / 2.0);
            let mut objs = vec![];
            for i in 0..len {
                for j in 0..len {
                    let translate = glam::Vec3::new(
                        i as f32 * trans_rate - offset,
                        j as f32 * trans_rate - offset,
                        0.0,
                    );
                    let color = glam::Vec4::ONE;
                    let obj = DrawObject::new(device, translate, Vec3::ONE * 0.1, color);
                    objs.push(obj);
                }
            }
            objs
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
