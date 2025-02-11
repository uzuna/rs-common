use mls_mpm::Sim;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use wasm_util::{error::*, info, time::AnimationTicker, util::get_document};
use web_sys::HtmlCanvasElement;
use wgpu::{InstanceDescriptor, SurfaceTarget};

/// モジュールの初期化処理
#[wasm_bindgen(start)]
pub fn init() -> Result<()> {
    wasm_util::panic::set_panic_hook();
    Ok(())
}

/// wasmのエントリーポイントとして定義
#[wasm_bindgen]
pub fn start() -> std::result::Result<(), JsValue> {
    spawn_local(async {
        let ctx = initialize_html().await.unwrap();
        let config = mls_mpm::SimConfig::new(100, 10);
        let mut sim = Sim::<f32>::init(config);
        let mut t = AnimationTicker::default();
        while let Ok(i) = t.tick().await {
            let dt_sec = i as f32 / 1000.0;

            let output = match ctx.surface.get_current_texture() {
                Ok(output) => output,
                Err(_) => {
                    info!("failed to get current texture");
                    continue;
                }
            };
            let view = output
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Render Encoder"),
                });
            sim.simulate(dt_sec);
            info!("tick {dt_sec:?}");
        }
    });
    Ok(())
}

struct WgpuContext {
    element: HtmlCanvasElement,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

async fn initialize_html() -> Result<WgpuContext> {
    // タイトルの設定
    let title = "WebAssemblyでMLS-MPMを実行";
    let title_element = get_document()?
        .create_element("title")
        .map_err(|_| JsError::new("failed to create title element"))?;
    title_element.set_inner_html(title);

    // canvasの設定
    let canvas = get_document()?
        .create_element("canvas")
        .map_err(|_| JsError::new("failed to create canvas element"))?;
    let canvas = canvas
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| JsError::new("failed to convert canvas element"))?;
    canvas.set_width(1024);
    canvas.set_height(768);

    let instance = wgpu::Instance::new(&InstanceDescriptor::from_env_or_default());
    let target = SurfaceTarget::Canvas(canvas.clone());
    let surface = instance.create_surface(target).unwrap();
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .unwrap();
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default(), None)
        .await
        .unwrap();

    Ok(WgpuContext {
        element: canvas,
        instance,
        surface,
        adapter,
        device,
        queue,
    })
}
