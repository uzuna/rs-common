use std::collections::VecDeque;

use app::{Context, BG_COLOR};
use egui::{Color32, CornerRadius, NumExt, Response};
use egui_plot::{Legend, Line, Plot, PlotPoints};

mod app;
mod gltf_view;
mod render;
mod tf;
mod ui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 720.0]),
        renderer: eframe::Renderer::Wgpu,
        depth_buffer: 32,
        ..Default::default()
    };

    eframe::run_native(
        "My egui App",
        options,
        Box::new(|cc| Ok(Box::new(CustomApp::new(cc)))),
    )
}

struct State {
    name: String,
    age: u32,
    next_title: Option<String>,
}

impl State {
    fn new() -> Self {
        Self {
            name: "Arthur".to_owned(),
            age: 42,
            next_title: None,
        }
    }

    fn fetch_title(&mut self) -> Option<String> {
        self.next_title.take()
    }
}

fn new_frame(bgcolor: Color32) -> egui::containers::Frame {
    egui::containers::Frame {
        inner_margin: egui::epaint::Margin::ZERO,
        outer_margin: egui::epaint::Margin::ZERO,
        corner_radius: CornerRadius::ZERO,
        stroke: egui::Stroke::default(),
        shadow: egui::epaint::Shadow::NONE,
        fill: bgcolor,
    }
}

struct CustomApp {
    state: State,
    ctx: Context,
    viewapp: gltf_view::ViewApp,

    line_demo: LineDemo,
}

impl CustomApp {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Self {
        let wgpu_render_state = cc
            .wgpu_render_state
            .as_ref()
            .expect("Failed to get WGPU render state");
        render::init(wgpu_render_state, 1.0);
        let ctx = Context::new(wgpu_render_state).expect("Failed to create context");

        Self {
            state: State::new(),
            ctx,
            viewapp: gltf_view::ViewApp::new(),
            line_demo: LineDemo::default(),
        }
    }
}

impl eframe::App for CustomApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // top level panel
        egui::CentralPanel::default()
            .frame(new_frame(BG_COLOR))
            .show(ctx, |ui| {
                self.ctx.shape(ui);
            });

        // SubWindow
        egui::Window::new("Spine Control")
            .default_width(320.0)
            .default_height(480.0)
            .resizable([true, false])
            .scroll(false)
            .show(ctx, |ui| {
                let age = &mut self.state.age;
                let name = &mut self.state.name;
                ui.horizontal(|ui| {
                    let name_label = ui.label("Your name: ");
                    // テキスト入力されてEnterで確定された場合にタイトル更新の例
                    let res = ui.text_edit_singleline(name).labelled_by(name_label.id);
                    if res.lost_focus() && ui.input(|i| i.key_down(egui::Key::Enter)) {
                        self.state.next_title = Some(format!("Hello, {name}"));
                    }
                });
                ui.add(egui::Slider::new(age, 0..=120).text("age"));
                if ui.button("Increment").clicked() {
                    *age += 1;
                }
                ui.label(format!("Hello '{name}', age {age}"));
                self.ctx.custom_painting(ui);
            });

        egui::Window::new("plot test")
            .default_width(300.0)
            .default_height(300.0)
            .resizable([true, false])
            .scroll(false)
            .show(ctx, |ui| self.line_demo.ui(ui));

        self.viewapp.update(ctx, frame);

        if let Some(title) = self.state.fetch_title() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // animation
    }
}

struct LineDemo {
    animate: bool,
    time: f64,
    show_axes: bool,
    show_grid: bool,
    plot_data: VecDeque<(f64, f64)>,
}

impl Default for LineDemo {
    fn default() -> Self {
        Self {
            animate: true,
            time: 0.0,
            show_axes: true,
            show_grid: true,
            plot_data: VecDeque::new(),
        }
    }
}

impl LineDemo {
    const LEN: usize = 256;
    fn rand<'a>(&self) -> Line<'a> {
        Line::new(
            "random + sin(time)",
            PlotPoints::from_iter(self.plot_data.iter().map(|(x, y)| {
                let x = *x;
                let y = *y;
                [x, y]
            })),
        )
        .color(Color32::from_rgb(100, 150, 250))
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> Response {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.animate, "Animate");
            ui.checkbox(&mut self.show_axes, "Show Axes");
            ui.checkbox(&mut self.show_grid, "Show Grid");
        });
        if self.animate {
            ui.ctx().request_repaint();
            self.time += ui.input(|i| i.unstable_dt).at_most(1.0 / 30.0) as f64;
            if self.plot_data.len() > Self::LEN {
                self.plot_data.pop_front();
            }
            self.plot_data
                .push_back((self.time, rand::random::<f64>() + self.time.sin()));
        };

        let plot = Plot::new("lines_demo")
            .legend(Legend::default())
            .show_axes(self.show_axes)
            .show_grid(self.show_grid)
            .center_y_axis(true);
        plot.show(ui, |plot_ui| {
            plot_ui.line(self.rand());
        })
        .response
    }
}
