use serde::{Deserialize, Serialize};
use tracing::{error, info};
use tracing_subscriber::prelude::*;

pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct HeaderState {
    is_open: bool,
    text: String,
}

impl HeaderState {
    const STORAGE_KEY: &str = "plotter-with-plugin-header";
    fn load(cc: &eframe::CreationContext<'_>) -> Self {
        // ストレージから状態を読み込む
        cc.storage
            .and_then(|s| eframe::get_value(s, Self::STORAGE_KEY))
            .unwrap_or_default()
    }

    fn save(&self, storage: &mut dyn eframe::Storage) {
        // ストレージに状態を保存する
        eframe::set_value(storage, Self::STORAGE_KEY, self);
    }
}

struct App {
    header: HeaderState,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let header = HeaderState::load(cc);
        Self { header }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // アプリケーションの状態を保存する
        self.header.save(storage);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().inner_margin(4))
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("v{APP_VERSION}"))
                        .on_hover_text("About this app");

                    ui.separator();

                    ui.toggle_value(&mut self.header.is_open, "Side Panel")
                        .on_hover_text("Toggle side panel visibility");
                });
            });

        egui::SidePanel::left("tools")
            .resizable(false)
            .exact_width(200.0)
            .show_animated(ctx, self.header.is_open, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.heading("PlotControl");
                    });

                    ui.separator();
                })
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("This is a plotter with plugin example.");
                ui.label("You can add plugins to extend functionality.");
            });

            // ここにプラグインのUIを追加することができます
        });
    }
}

fn init() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_hal=warn".into()),
        )
        .init();
}

fn main() -> anyhow::Result<()> {
    init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        ..Default::default()
    };
    match eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc))))) {
        Ok(_) => {
            info!("exit application");
            Ok(())
        }
        Err(e) => {
            error!("error: {:?}", e);
            Err(anyhow::anyhow!("eframe error: {:?}", e))
        }
    }
}
