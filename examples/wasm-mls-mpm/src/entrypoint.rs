use mls_mpm::ElasticConfig;
use nalgebra::Vector2;
use rand::Rng;
use wasm_util::util::get_performance;
use wgpu_shader::{particle, prelude::*, uniform::UniformBuffer, vertex::VertexBufferInstanced};
use winit::{
    event::*,
    event_loop::EventLoop,
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};

use wasm_bindgen::prelude::*;

use crate::state;

fn update_vertex(
    vertices: &mut [particle::shader::VertexInput],
    particles: &[mls_mpm::Particle<f32>],
) {
    for (v, p) in vertices.iter_mut().zip(particles.iter()) {
        v.position = [p.pos.x, p.pos.y, 0.0].into();
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
        let mut verts = vec![particle::shader::VertexInput::default(); particles.len()];
        update_vertex(&mut verts, particles);
        verts
    };

    // Context::new uses async code, so we're going to wait for it to finish
    let mut state = state::State::new(&window).await;
    let mut surface_configured = false;
    let p = get_performance()?;
    let start = p.now();
    let mut last = p.now();

    // init render uniform

    let u_w = particle::shader::Window {
        resolution: [width as f32, height as f32].into(),
        pixel_size: [10.0, 10.0].into(),
    };

    let uniform = UniformBuffer::new(state.device(), u_w);

    let pipe = particle::Pipeline::new(state.device(), state.config(), &uniform);

    // init vertex
    let vb = VertexBufferInstanced::new(state.device(), &verts, Some("Vertex Buffer"));

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
                                let _elapsed = p.now() - start;
                                let dt = (p.now() - last) / 1000.0;
                                last = p.now();

                                // This tells winit that we want another frame after this one
                                state.window().request_redraw();

                                if !surface_configured {
                                    return;
                                }

                                // update simulation
                                sim.simulate(dt as f32);
                                let particles = sim.get_particles_mut();
                                update_vertex(&mut verts, particles);
                                vb.update(state.queue(), &verts);

                                match pipe.render(&state, &vb) {
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
                }
                _ => {}
            }
        })
        .map_err(|e| JsError::new(&format!("{:?}", e)))?;
    Ok(())
}
