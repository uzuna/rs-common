use std::time::Instant;

use iced::widget::{button, column, container, horizontal_space, row, slider, text, Column};
use iced::{border, window, Alignment, Element, Font, Subscription, Task, Theme};
use scene::systeminfo::{self, SystemInfoMsg};
use scene::{ltm, title_text};
use tracing::info;

pub mod scene;
pub mod server;

pub fn main() -> iced::Result {
    init();
    let s = iced::settings::Settings {
        default_font: Font::with_name("Consolas"),
        ..iced::settings::Settings::default()
    };
    iced::application(TitleView, State::update, State::view)
        .settings(s)
        .subscription(State::subscription)
        .run_with(|| {
            // 初回起動時のタスクを指定
            let mut state = State::default();
            let task = state.update(Message::StartUp);
            (state, task)
        })
}

struct State {
    screen: Screen,
    value: i64,
    slider: i32,
    title_text: title_text::Scene,
    server_text: String,
    si: systeminfo::Scene,
    handle: Option<server::ThreadHandles>,
    ch: Option<server::UiCh>,
    store: Option<scene::ltm::Store>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            value: 0,
            slider: 0,
            title_text: title_text::Scene::new(),
            server_text: "".to_string(),
            screen: Screen::Welcome,
            si: systeminfo::Scene::new(),
            handle: None,
            ch: None,
            store: None,
        }
    }
}

/// 表示内容の切り替え
enum Screen {
    Welcome,
}

/// アプリケーションのタイトルを表示するための構造体
struct TitleView;

impl iced::application::Title<State> for TitleView {
    fn title(&self, state: &State) -> String {
        // 現在のコンテンツをもとに表示
        state.title_text.title().to_string()
    }
}
#[derive(Debug, Clone)]
enum Message {
    StartUp,
    SystemInfo(SystemInfoMsg),
    ServerStart,
    ServerStop,
    Increment,
    Decrement,
    SliderChanged(i32),
    #[allow(dead_code)]
    Tick(Instant),
    TitleText(scene::title_text::TitleTextMsg),
}

impl From<scene::title_text::TitleTextMsg> for Message {
    fn from(msg: scene::title_text::TitleTextMsg) -> Self {
        Message::TitleText(msg)
    }
}

impl State {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StartUp => {
                info!("Start up");
                let store = ltm::Store::new().unwrap();
                self.title_text
                    .update(&store, title_text::TitleTextMsg::Load);
                self.store = Some(store);
            }
            Message::SystemInfo(s) => return self.si.update(s).map(Message::SystemInfo),
            Message::ServerStart => {
                if self.handle.is_none() {
                    let (h, ch) = server::spawn();
                    self.handle = Some(h);
                    self.ch = Some(ch);
                }
            }
            Message::ServerStop => {
                if let Some(h) = self.handle.take() {
                    h.process.cancel();
                    h.handle.join().unwrap().unwrap();
                }
            }
            Message::Increment => {
                self.value += 1;
            }
            Message::Decrement => {
                self.value -= 1;
            }
            Message::SliderChanged(value) => {
                self.slider = value;
            }
            Message::Tick(_) => {
                if let Some(ch) = &mut self.ch {
                    while let Ok(msg) = ch.rx.try_recv() {
                        match msg {
                            server::Response::Tick(elapsed) => {
                                self.server_text = format!("Elapsed: {:?}", elapsed);
                            }
                        }
                    }
                }
            }
            Message::TitleText(msg) => {
                self.title_text.update(self.store.as_ref().unwrap(), msg);
            }
        };
        Task::none()
    }

    fn test_view(&self) -> Column<Message> {
        let header = container(
            row![
                // square(40),
                horizontal_space(),
                "Header!",
                horizontal_space(),
                // square(40),
            ]
            .padding(10)
            .align_y(Alignment::Start),
        )
        .align_left(200)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();

            container::Style::default()
                .border(border::color(palette.background.strong.color).width(1))
        })
        .align_x(Alignment::Start);

        let body = column![
            button("Increment").on_press(Message::Increment),
            text(self.value).size(50),
            button("Decrement").on_press(Message::Decrement),
            self.title_text.view(),
            slider(-100..=100, self.slider, Message::SliderChanged),
            text(self.slider).size(14),
            button("Start server").on_press(Message::ServerStart),
            button("Stop server").on_press(Message::ServerStop),
            text(&self.server_text).size(14),
        ]
        .spacing(20)
        .width(200);
        column![header, body]
    }

    fn view(&self) -> Element<Message> {
        match self.screen {
            Screen::Welcome => {
                let content = column![
                    button("Load system information")
                        .on_press(Message::SystemInfo(SystemInfoMsg::LoadSystemInfo)),
                    // button("Test").on_press(Message::LoadSystemInfo),
                    // button("Settings").on_press(Message::Settings),
                    self.test_view(),
                    self.si.view(),
                ]
                .spacing(20)
                .align_x(Alignment::Center);

                content.into()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        window::frames().map(Message::Tick)
    }
}

fn init() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}
