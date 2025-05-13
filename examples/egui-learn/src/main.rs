use shader::Context;

mod gltf_view;
mod render;
mod shader;
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

struct CustomApp {
    state: State,
    ctx: Context,
    viewapp: gltf_view::ViewApp,
}

impl CustomApp {
    pub fn new<'a>(cc: &'a eframe::CreationContext<'a>) -> Self {
        let wgpu_render_state = cc.wgpu_render_state.as_ref().unwrap();
        let device = &wgpu_render_state.device;
        let (ctx, rr) = Context::new(device, wgpu_render_state.target_format);
        wgpu_render_state
            .renderer
            .write()
            .callback_resources
            .insert(rr);

        render::init(wgpu_render_state, 1.0);

        Self {
            state: State::new(),
            ctx,
            viewapp: gltf_view::ViewApp::new(),
        }
    }
}

impl eframe::App for CustomApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // top level panel
        egui::CentralPanel::default().show(ctx, |ui| {
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

        self.viewapp.update(ctx, frame);

        if let Some(title) = self.state.fetch_title() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}
