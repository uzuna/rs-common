use std::time::Duration;

use wgpu_shader::{particle, prelude::*};
use winit::{
    event::*,
    event_loop::EventLoopBuilder,
    keyboard::{KeyCode, PhysicalKey},
    platform::x11::EventLoopBuilderExtX11,
    window::WindowBuilder,
};

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
    let u_w = particle::shader::Window {
        resolution: [800.0, 600.0, 1.0, 0.0].into(),
    };
    let mut uniform = particle::Unif::new(state.device(), u_w);

    let pipe = particle::Pipeline::new(state.device(), state.config(), &uniform);

    // init vertex
    let mut verts = vec![];
    for x in 0..10 {
        for y in 0..10 {
            verts.push(particle::shader::VertexInput {
                position: [x as f32 * 0.1 - 0.5, y as f32 * 0.1 - 0.5, 0.0].into(),
                color: [1.0, 0.0, 0.0].into(),
            });
        }
    }

    let vb = particle::Vert::new(state.device(), &verts, Some("Vertex Buffer"));

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
