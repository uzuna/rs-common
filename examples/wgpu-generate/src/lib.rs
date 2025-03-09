use std::{iter, ops::Range, time::Duration};

use encase::{internal::WriteInto, ShaderType, UniformBuffer};
use shader::VertexInput;
use state::State;
use wgpu::util::DeviceExt;
use winit::{
    event::*,
    event_loop::EventLoopBuilder,
    keyboard::{KeyCode, PhysicalKey},
    platform::x11::EventLoopBuilderExtX11,
    window::WindowBuilder,
};

pub mod shader;
pub mod state;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run(timeout: Option<Duration>) {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Info).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }
    let event_loop = EventLoopBuilder::new()
        .with_any_thread(true)
        .build()
        .unwrap();
    let window = WindowBuilder::new().build(&event_loop).unwrap();

    #[cfg(target_arch = "wasm32")]
    {
        // Winit prevents sizing with CSS, so we have to set
        // the size manually when on web.
        use winit::dpi::PhysicalSize;

        use winit::platform::web::WindowExtWebSys;
        web_sys::window()
            .and_then(|win| win.document())
            .and_then(|doc| {
                let dst = doc.get_element_by_id("wasm-example")?;
                let canvas = web_sys::Element::from(window.canvas()?);
                dst.append_child(&canvas).ok()?;
                Some(())
            })
            .expect("Couldn't append canvas to document body.");

        let _ = window.request_inner_size(PhysicalSize::new(450, 400));
    }

    // State::new uses async code, so we're going to wait for it to finish
    let mut state = state::State::new(&window).await;
    let mut surface_configured = false;
    let start = std::time::Instant::now();

    // init render uniform
    let u_w = shader::Window {
        resolution: [800.0, 600.0, 1.0, 0.0].into(),
    };
    let mut uniform = Unif::new(state.device(), u_w);

    let ctx = RenderContext::new(state.device(), state.config(), &uniform);

    // init vertex
    let verts = vec![VertexInput::new(); 10];
    let vb = Vert::new(state.device(), &verts, Some("Vertex Buffer"));

    event_loop
        .run(move |event, control_flow| {
            match event {
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == state.window().id() => {
                    if !state.input(event) {
                        // UPDATED!
                        match event {
                            WindowEvent::CloseRequested
                            | WindowEvent::KeyboardInput {
                                event:
                                    KeyEvent {
                                        state: ElementState::Pressed,
                                        physical_key: PhysicalKey::Code(KeyCode::Escape),
                                        ..
                                    },
                                ..
                            } => control_flow.exit(),
                            WindowEvent::Resized(physical_size) => {
                                log::info!("physical_size: {physical_size:?}");
                                surface_configured = true;
                                state.resize(*physical_size);
                            }
                            WindowEvent::RedrawRequested => {
                                // This tells winit that we want another frame after this one
                                state.window().request_redraw();

                                if !surface_configured {
                                    return;
                                }
                                uniform.set(state.queue(), &u_w);

                                match ctx.render(&mut state, &vb) {
                                    Ok(_) => {}
                                    // Reconfigure the surface if it's lost or outdated
                                    Err(
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                                    ) => state.resize(state.size()),
                                    // The system is out of memory, we should probably quit
                                    Err(
                                        wgpu::SurfaceError::OutOfMemory | wgpu::SurfaceError::Other,
                                    ) => {
                                        log::error!("OutOfMemory");
                                        control_flow.exit();
                                    }

                                    // This happens when the a frame takes too long to present
                                    Err(wgpu::SurfaceError::Timeout) => {
                                        log::warn!("Surface timeout")
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    if let Some(timeout) = timeout {
                        if start.elapsed() > timeout {
                            control_flow.exit();
                        }
                    }
                }
                _ => {}
            }
        })
        .unwrap();
}

impl VertexInput {
    fn new() -> Self {
        Self {
            position: [0.0, 0.0, 0.0].into(),
            color: [1.0, 0.0, 0.0].into(),
        }
    }
}

pub struct RenderContext {
    pipe: wgpu::RenderPipeline,
    bind_group: shader::bind_groups::BindGroup0,
}

impl RenderContext {
    /// パイプラインの構築
    pub fn new(
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
        uniform_buffer: &Unif<shader::Window>,
    ) -> Self {
        let shader = shader::create_shader_module(device);

        let render_pipeline_layout = shader::create_pipeline_layout(device);
        let fs_target = [Some(wgpu::ColorTargetState {
            format: config.format,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent::REPLACE,
                alpha: wgpu::BlendComponent::REPLACE,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: shader::vertex_state(
                &shader,
                &shader::vs_main_entry(wgpu::VertexStepMode::Vertex),
            ),
            fragment: Some(shader::fragment_state(
                &shader,
                &shader::fs_main_entry(fs_target),
            )),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::default(),
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // uniform bufferのバインドグループの作成
        let bind_group = shader::bind_groups::BindGroup0::from_bindings(
            device,
            shader::bind_groups::BindGroupLayout0 {
                uw: uniform_buffer.buffer().as_entire_buffer_binding(),
            },
        );
        Self {
            pipe: pipeline,
            bind_group,
        }
    }

    fn render(
        &self,
        state: &mut State,
        buf: &Vert<shader::VertexInput>,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = state.surface().get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = state
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.pipe);
            self.bind_group.set(&mut render_pass);
            // shaderが6頂点を描画する仕様になっている
            buf.draw(&mut render_pass, 0..6);
        }

        state.queue().submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

pub struct Unif<U> {
    buffer: wgpu::Buffer,
    ub: UniformBuffer<Vec<u8>>,

    _phantom: std::marker::PhantomData<U>,
}

impl<U> Unif<U>
where
    U: ShaderType + WriteInto,
{
    pub fn new(device: &wgpu::Device, u: U) -> Self {
        let mut ub = UniformBuffer::new(Vec::new());
        ub.write(&u).expect("Failed to write uniform buffer");
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: ub.as_ref(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            buffer,
            ub,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn set(&mut self, queue: &wgpu::Queue, u: &U) {
        self.ub.write(u).expect("Failed to write uniform buffer");
        queue.write_buffer(&self.buffer, 0, self.ub.as_ref());
    }

    pub fn uniform_buffer(&self) -> &UniformBuffer<Vec<u8>> {
        &self.ub
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

pub struct Vert<V> {
    pub buf: wgpu::Buffer,
    vert_len: usize,
    phantom: std::marker::PhantomData<V>,
}

impl<V> Vert<V>
where
    V: bytemuck::Pod,
{
    pub fn new(device: &wgpu::Device, verts: &[V], label: Option<&str>) -> Self {
        let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label,
            contents: bytemuck::cast_slice(verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            buf,
            vert_len: verts.len(),
            phantom: std::marker::PhantomData,
        }
    }

    pub fn draw(&self, rpass: &mut wgpu::RenderPass, vert_range: Range<u32>) {
        rpass.set_vertex_buffer(0, self.buf.slice(..));
        rpass.draw(vert_range, 0..self.vert_len as u32);
    }
}
