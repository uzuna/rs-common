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
        particle::*, uniform::UniformBuffer, vertex::VertexBufferInstanced, WgpuContext,
    };

    use super::Timestamp;

    #[allow(dead_code)]
    pub struct Context {
        pipe: Pipeline,
        uniform: UniformBuffer<shader::Window>,
        vertexies: Vec<shader::VertexInput>,
        vb: VertexBufferInstanced<shader::VertexInput>,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let u_w = shader::Window {
                resolution: [800.0, 600.0, 1.0, 0.0].into(),
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

            let vb = VertexBufferInstanced::new(state.device(), &verts, Some("Vertex Buffer"));

            Self {
                pipe,
                uniform,
                vertexies: verts,
                vb,
            }
        }

        pub fn update(&mut self, _state: &impl WgpuContext, _ts: &Timestamp) {}

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            self.pipe.render(state, &self.vb)
        }
    }
}

pub mod introduction {
    use glam::Vec3;
    use wgpu_shader::introduction::shader::VertexInput;
    use wgpu_shader::vertex::VertexBuffer;
    use wgpu_shader::{introduction::*, WgpuContext};

    use super::Timestamp;

    const TRIANGLE: &[VertexInput] = &[
        VertexInput::new(Vec3::new(0.0, 0.5, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        VertexInput::new(Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.0, 1.0, 0.0)),
        VertexInput::new(Vec3::new(0.5, -0.5, 0.0), Vec3::new(0.0, 0.0, 1.0)),
    ];

    const TRIANGLE_INDEXIES: &[u16] = &[0, 1, 2];

    const PENTAGON: &[VertexInput] = &[
        VertexInput::new(
            Vec3::new(-0.0868241, 0.49240386, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ),
        VertexInput::new(
            Vec3::new(-0.49513406, 0.06958647, 0.0),
            Vec3::new(0.5, 0.0, 0.5),
        ),
        VertexInput::new(
            Vec3::new(-0.21918549, -0.44939706, 0.0),
            Vec3::new(0.5, 1.0, 0.5),
        ),
        VertexInput::new(
            Vec3::new(0.35966998, -0.3473291, 0.0),
            Vec3::new(0.0, 0.0, 0.5),
        ),
        VertexInput::new(
            Vec3::new(0.44147372, 0.2347359, 0.0),
            Vec3::new(0.5, 0.0, 1.0),
        ),
    ];

    const PENTAGON_INDEXIES: &[u16] = &[0, 1, 4, 1, 2, 4, 2, 3, 4];

    #[allow(dead_code)]
    pub struct Context {
        pipe: Pipeline,
        vb: VertexBuffer<VertexInput>,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let mut pipe = Pipeline::new(state.device(), config);
            pipe.set_bg_color(super::BG_COLOR);
            let vb = Self::pentagon(state);

            Self { pipe, vb }
        }

        fn triangle(state: &impl WgpuContext) -> VertexBuffer<VertexInput> {
            VertexBuffer::new(state.device(), TRIANGLE, TRIANGLE_INDEXIES)
        }

        fn pentagon(state: &impl WgpuContext) -> VertexBuffer<VertexInput> {
            VertexBuffer::new(state.device(), PENTAGON, PENTAGON_INDEXIES)
        }

        pub fn update(&mut self, state: &impl WgpuContext, ts: &Timestamp) {
            // 3角と5角の表示を切り替える
            if ts.elapsed.as_secs_f32().sin() > 0.0 {
                self.vb = Self::triangle(state);
            } else {
                self.vb = Self::pentagon(state);
            }
        }

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            self.pipe.render(state, &self.vb)
        }
    }
}

pub mod tutorial {

    use std::path::Path;

    use glam::{Vec2, Vec3, Vec4};
    use nalgebra::{Rotation3, Translation3, Vector3};
    use wgpu_shader::{
        prelude::*,
        tutorial::{
            self,
            shader::{InstanceInput, VertexInput},
            *,
        },
        uniform::UniformBuffer,
        vertex::{VertexBuffer, ViBuffer},
        WgpuContext,
    };

    use crate::{
        camera::{Camera, CameraController},
        resources::ModelData,
    };

    use super::BG_COLOR;

    const PENTAGON: &[VertexInput] = &[
        VertexInput::new(
            Vec3::new(-0.0868241, 0.49240386, 0.0),
            Vec2::new(0.4131759, 0.99240386),
        ),
        VertexInput::new(
            Vec3::new(-0.49513406, 0.06958647, 0.0),
            Vec2::new(0.0048659444, 0.56958647),
        ),
        VertexInput::new(
            Vec3::new(-0.21918549, -0.44939706, 0.0),
            Vec2::new(0.28081453, 0.05060294),
        ),
        VertexInput::new(
            Vec3::new(0.35966998, -0.3473291, 0.0),
            Vec2::new(0.85967, 0.1526709),
        ),
        VertexInput::new(
            Vec3::new(0.44147372, 0.2347359, 0.0),
            Vec2::new(0.9414737, 0.7347359),
        ),
    ];

    const PENTAGON_INDEXIES: &[u16] = &[0, 1, 4, 1, 2, 4, 2, 3, 4];

    fn instances() -> Vec<InstanceInput> {
        let mut instances = vec![];
        let step = 0.6;
        let offset = 5.0 * -0.5;
        for z in 0..10 {
            for x in 0..10 {
                let pos: nalgebra::Matrix<
                    f32,
                    nalgebra::Const<3>,
                    nalgebra::Const<1>,
                    nalgebra::ArrayStorage<f32, 3, 1>,
                > = Vector3::new(x as f32 * step + offset, 0.0, z as f32 * step + offset);
                let rot = Rotation3::from_euler_angles(
                    (x as f32 * 0.1).to_degrees(),
                    (z as f32 * 0.1).to_degrees(),
                    0.0,
                );
                instances.push(InstanceInput::from(glam::Mat4::from(
                    Translation3::from(pos).to_homogeneous() * rot.to_homogeneous(),
                )));
            }
        }
        instances
    }

    fn into_camuni(cam: &Camera) -> shader::Camera {
        let pos = cam.pos();
        shader::Camera {
            view_pos: Vec4::new(pos.x, pos.y, pos.z, 1.0),
            view_proj: cam.build_view_projection_matrix().into(),
        }
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

    #[allow(dead_code)]
    pub struct Context {
        pipe_render: Pipeline,
        pipe_light: LightRenderPipeline,
        cam: Camera,
        cc: CameraController,
        cb_render: UniformBuffer<shader::Camera>,
        cb_light: UniformBuffer<light::Camera>,
        lb_light: UniformBuffer<light::Light>,
        m0: tutorial::Model,
        model: Model,
    }

    impl Context {
        pub fn new(
            state: &impl WgpuContext,
            config: &wgpu::SurfaceConfiguration,
            assets_dir: &Path,
        ) -> Self {
            let texture_path = assets_dir.join("webgpu.png");
            let tex = load_texture(state, &texture_path).expect("Failed to load texture");

            let cam = Camera::with_aspect(config.width as f32 / config.height as f32);
            let cc = CameraController::new(0.01);
            let cam_mat = into_camuni(&cam);
            let cb_render = UniformBuffer::new(state.device(), cam_mat);

            let pipe_render = Pipeline::new(state.device(), config, &cb_render);

            let vb = Self::pentagon(state);

            let m0 = tutorial::Model::new(state.device(), tex, vb);
            let model = Self::load_model(state, &assets_dir.join("models/cube/cube.obj"));

            let cam_mat: light::Camera = into_camuni(&cam).into();
            let cb_light = UniformBuffer::new(state.device(), cam_mat);
            let lb_light = UniformBuffer::new(state.device(), create_light());
            let pipe_light = LightRenderPipeline::new(state.device(), config, &cb_light, &lb_light);

            Self {
                pipe_render,
                pipe_light,
                cam,
                cc,
                cb_render,
                cb_light,
                lb_light,
                m0,
                model,
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

        fn pentagon(state: &impl WgpuContext) -> ViBuffer<VertexInput, InstanceInput> {
            ViBuffer::new(state.device(), PENTAGON, PENTAGON_INDEXIES, &instances())
        }

        pub fn input(&mut self, event: &winit::event::WindowEvent) -> bool {
            self.cc.process_events(event)
        }

        pub fn resize(&mut self, state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) {
            self.cam
                .set_aspect(config.width as f32 / config.height as f32);
            self.cb_render.set(state.queue(), &into_camuni(&self.cam));
        }

        pub fn update(&mut self, state: &impl WgpuContext, _ts: &super::Timestamp) {
            self.cc.update_camera(&mut self.cam);
            let cb = into_camuni(&self.cam);
            self.cb_render.set(state.queue(), &cb);
            self.cb_light.set(state.queue(), &cb.into());
        }

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            render(state, BG_COLOR, state.depth(), |rp| {
                let r = &self.pipe_light;
                rp.set_pipeline(r.pipe());
                r.bg0.set(rp);
                r.bg1.set(rp);
                self.model.mesh.vb.draw(rp, 0..1);

                let r = &self.pipe_render;
                rp.set_pipeline(r.pipe());
                r.bg1.set(rp);
                self.m0.draw(rp);
                self.model.material.bg.set(rp);
                self.model.mesh.vb.draw(rp, 0..1);
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
