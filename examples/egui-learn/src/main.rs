fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        renderer: eframe::Renderer::Wgpu,
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
}

impl CustomApp {
    pub fn new<'a>(_cc: &'a eframe::CreationContext<'a>) -> Self {
        Self {
            state: State::new(),
        }
    }
}

impl eframe::App for CustomApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let age = &mut self.state.age;
            let name = &mut self.state.name;
            ui.heading("My egui Application");
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
        });

        if let Some(title) = self.state.fetch_title() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}
