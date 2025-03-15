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

pub mod texture {
    use glam::{Vec2, Vec3};
    use wgpu_shader::{
        prelude::*, texture::shader::VertexInput, texture::*, vertex::VertexBuffer, WgpuContext,
    };

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

    #[allow(dead_code)]
    pub struct Context {
        pipe: Pipeline,
        vb: VertexBuffer<VertexInput>,
        tx: TextureInst,
    }

    impl Context {
        pub fn new(state: &impl WgpuContext, config: &wgpu::SurfaceConfiguration) -> Self {
            let tx = load_texture(state);
            let mut pipe = Pipeline::new(state.device(), config, &tx);
            pipe.set_bg_color(super::BG_COLOR);
            let vb = Self::pentagon(state);
            Self { pipe, vb, tx }
        }

        fn pentagon(state: &impl WgpuContext) -> VertexBuffer<VertexInput> {
            VertexBuffer::new(state.device(), PENTAGON, PENTAGON_INDEXIES)
        }

        pub fn update(&mut self, _state: &impl WgpuContext, _ts: &super::Timestamp) {}

        pub fn render(&self, state: &impl WgpuContext) -> Result<(), wgpu::SurfaceError> {
            self.pipe.render(state, &self.vb)
        }
    }

    fn load_texture(state: &impl WgpuContext) -> TextureInst {
        let img = include_bytes!("../../assets/webgpu.png");
        let img = image::load_from_memory(img).unwrap();
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

        inst
    }
}
