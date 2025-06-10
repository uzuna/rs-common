use egui::ComboBox;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use tracing_subscriber::prelude::*;

use crate::{plot::SignalProcess, plugin::PluginLoader};

pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod plot;
pub mod plugin;
pub mod wrun;

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
    sp: SignalProcess,
    pl: PluginLoader,
    error: Option<String>,
    selected_plugin: usize,
    param_key: String,
    param_value: String,
    set_queue: Vec<(String, String, String)>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let header = HeaderState::load(cc);
        let sp = SignalProcess::new(15.0, 128.0, std::time::Duration::from_secs(10));
        let pl = plugin::PluginLoader::default();
        Self {
            header,
            sp,
            pl,
            error: None,
            selected_plugin: 0,
            param_key: String::new(),
            param_value: String::new(),
            set_queue: Vec::new(),
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // アプリケーションの状態を保存する
        self.header.save(storage);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sp
            .update(std::time::Duration::from_millis(16))
            .expect("Failed to update signal process");
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
                });

                ui.separator();

                if ui.button("Open Plugin File..").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        match path.extension() {
                            Some(ext) if ext == "wasm" => {}
                            _ => {
                                ui.label("Not a WASM file");
                                return;
                            }
                        }
                        let buffer = std::fs::read(&path).expect("Failed to read plugin file");
                        match self.pl.load_plugin(&buffer) {
                            Ok(mut plugin) => {
                                info!(
                                    "Plugin loaded: {}",
                                    plugin.name().expect("Failed to get plugin name")
                                );
                                self.sp.add_plugin(plugin);
                            }
                            Err(e) => {
                                self.error = Some(format!("Failed to load file: {}", e));
                            }
                        }
                    }
                    ui.separator();
                }

                if let Some(error) = &self.error {
                    ui.label(error);

                    ui.separator();
                }
                // parameterの設定UI
                ui.label("Plugin Parameters");
                // comboboxでプラグインを選択
                let plugins = self.sp.plugins().iter().enumerate().collect::<Vec<_>>();
                ComboBox::new("plugin_selector", "")
                    .selected_text(
                        plugins
                            .iter()
                            .find(|(a, _)| a == &self.selected_plugin)
                            .map_or("<Select Plugin>", |(_, name)| name.0),
                    )
                    .show_ui(ui, |ui| {
                        for (i, plugin) in &plugins {
                            ui.selectable_value(&mut self.selected_plugin, *i, plugin.0);
                        }
                    });
                // key-valueペアの設定
                ui.horizontal(|ui| {
                    ui.label("Key:");
                    ui.text_edit_singleline(&mut self.param_key);
                });
                ui.horizontal(|ui| {
                    ui.label("Value:");
                    ui.text_edit_singleline(&mut self.param_value);
                    let res = ui.add(egui::TextEdit::singleline(&mut self.param_value));
                    if res.lost_focus() && ui.input(|i| i.key_down(egui::Key::Enter)) {
                        let (plugin_name, param_key, param_value) = (
                            self.sp.plugins().keys().nth(self.selected_plugin).unwrap(),
                            self.param_key.clone(),
                            self.param_value.clone(),
                        );
                        info!(
                            "Setting parameter {}={} for plugin {}",
                            param_key, param_value, plugin_name
                        );
                        self.set_queue
                            .push((plugin_name.clone(), param_key, param_value));
                    }
                });
                if ui.button("Set Parameter").clicked() {
                    let plugin_name = self.sp.plugins().keys().nth(self.selected_plugin).unwrap();
                    info!("Setting parameter for plugin {}", plugin_name);
                    self.set_queue.push((
                        plugin_name.clone(),
                        self.param_key.clone(),
                        self.param_value.clone(),
                    ));
                }
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            // ここにプラグインのUIを追加することができます
            self.sp.plot(ui);
        });

        // プラグインのパラメータを設定する
        // uiせっていのとちゅ
        for (target, key, value) in self.set_queue.drain(..) {
            if let Err(e) = self.sp.set_param(&target, &key, &value) {
                self.error = Some(format!("Failed to set parameter: {}", e));
            } else {
                info!("Set parameter {}={} for plugin {}", key, value, target);
            }
        }
        ctx.request_repaint(); // 定期的に再描画を要求
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
