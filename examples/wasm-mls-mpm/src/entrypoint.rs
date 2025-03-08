use std::iter;

use mls_mpm::ElasticConfig;
use nalgebra::Vector2;
use rand::Rng;
use wasm_util::util::get_performance;
use winit::{
    event::*,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowBuilder},
};

use wasm_bindgen::prelude::*;

use crate::shader::{self, Uniform, Vertex};

struct Context<'a> {
    surface: wgpu::Surface<'a>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    render_pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    buf: shader::VertexBuffer,
    unibuf: shader::UniformBuffer,
    uniform: Uniform,
    // The window must be declared after the surface so
    // it gets dropped after it as the surface contains
    // unsafe references to the window's resources.
    window: &'a Window,
}

impl<'a> Context<'a> {
    async fn new(window: &'a Window, vertices: &[Vertex], uniform: Uniform) -> Context<'a> {
        let size = window.inner_size();

        // The instance is a handle to our GPU
        // BackendBit::PRIMARY => Vulkan + Metal + DX12 + Browser WebGPU
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::GL,
            ..Default::default()
        });

        let surface = instance.create_surface(window).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    // WebGL doesn't support all of wgpu's features, so if
                    // we're building for the web we'll have to disable some.
                    required_limits: if cfg!(target_arch = "wasm32") {
                        wgpu::Limits::downlevel_webgl2_defaults()
                    } else {
                        wgpu::Limits::default()
                    },
                    memory_hints: Default::default(),
                },
                // Some(&std::path::Path::new("trace")), // Trace path
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        // Shader code in this tutorial assumes an Srgb surface texture. Using a different
        // one will result all the colors comming out darker. If you want to support non
        // Srgb surfaces, you'll need to account for that when drawing to the frame.
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            desired_maximum_frame_latency: 2,
            view_formats: vec![],
        };
        let unibuf = shader::UniformBuffer::new(&device);
        let (render_pipeline, bind_group) = shader::render_pipeline(&device, &config, &unibuf);
        let buf = shader::VertexBuffer::new(&device, vertices);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            bind_group,
            buf,
            unibuf,
            uniform,
            window,
        }
    }

    fn window(&self) -> &Window {
        self.window
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    #[allow(unused_variables)]
    fn input(&mut self, event: &WindowEvent) -> bool {
        false
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            self.unibuf.update(&self.queue, self.uniform);
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

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            self.buf.draw(&mut render_pass);
        }

        self.queue.submit(iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}

fn update_vertex(vertices: &mut [Vertex], particles: &[mls_mpm::Particle<f32>]) {
    for (v, p) in vertices.iter_mut().zip(particles.iter()) {
        v.position = [p.pos.x, p.pos.y, 0.0];
    }
}

#[wasm_bindgen]
pub struct RunConfig {
    num_particles: usize,
    num_subdiv: usize,
    gravity_y: f32,
}

#[wasm_bindgen]
impl RunConfig {
    #[wasm_bindgen(constructor)]
    pub fn new(num_particles: usize, num_subdiv: usize, gravity_y: f32) -> Self {
        Self {
            num_particles,
            num_subdiv,
            gravity_y,
        }
    }
}

#[wasm_bindgen]
pub async fn run(c: RunConfig) -> Result<(), JsError> {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).expect("Couldn't initialize logger");

    let event_loop = EventLoop::new()?;
    let window = WindowBuilder::new().build(&event_loop)?;
    let (width, height) = (450, 400);
    let u = Uniform {
        resolution: [width as f32, height as f32, 1.0, 0.0],
    };

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

        let _ = window.request_inner_size(PhysicalSize::new(width, height));
    }

    let mut sim = mls_mpm::Sim::<f32>::new(mls_mpm::SimConfig::new(
        c.num_particles,
        c.num_subdiv,
        2.0,
        Vector2::new(0.0, c.gravity_y),
        ElasticConfig::<f32>::default(),
    ));
    // initialize position
    let pos_range = -0.5..0.5;
    let vel_range = -4.0..4.0;
    let mut verts = {
        let mut rng = rand::rngs::OsRng;
        let particles = sim.get_particles_mut();
        for p in particles.iter_mut() {
            p.pos = Vector2::new(
                rng.gen_range(pos_range.clone()),
                rng.gen_range(pos_range.clone()),
            );
            p.vel = Vector2::new(
                rng.gen_range(vel_range.clone()),
                rng.gen_range(vel_range.clone()),
            );
        }
        let mut verts = vec![Vertex::default(); particles.len()];
        update_vertex(&mut verts, particles);
        verts
    };

    // Context::new uses async code, so we're going to wait for it to finish
    let mut ctx = Context::new(&window, &verts, u).await;
    let mut surface_configured = false;
    let p = get_performance()?;
    let start = p.now();
    let mut last = p.now();

    event_loop
        .run(move |event, control_flow| {
            match event {
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == ctx.window().id() => {
                    if !ctx.input(event) {
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
                                ctx.resize(*physical_size);
                            }
                            WindowEvent::RedrawRequested => {
                                let _elapsed = p.now() - start;
                                let dt = (p.now() - last) / 1000.0;
                                last = p.now();

                                // This tells winit that we want another frame after this one
                                ctx.window().request_redraw();

                                if !surface_configured {
                                    return;
                                }

                                sim.simulate(dt as f32);
                                let particles = sim.get_particles_mut();
                                update_vertex(&mut verts, particles);
                                ctx.buf.update_vertices(&ctx.queue, &verts);
                                match ctx.render() {
                                    Ok(_) => {}
                                    // Reconfigure the surface if it's lost or outdated
                                    Err(
                                        wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated,
                                    ) => ctx.resize(ctx.size),
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
                }
                _ => {}
            }
        })
        .map_err(|e| JsError::new(&format!("{:?}", e)))?;
    Ok(())
}
