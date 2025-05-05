use iced::widget::{button, column, slider, text, text_input, Column};
use iced::{Center, Font};

const ICON_FONT: Font = Font::with_name("Consolas");

pub fn main() -> iced::Result {
    let s = iced::settings::Settings {
        default_font: ICON_FONT,
        ..iced::settings::Settings::default()
    };
    iced::application(TitleView, State::update, State::view)
        .settings(s)
        .run()
}

struct State {
    value: i64,
    slider: i32,
    content: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            value: 0,
            slider: 0,
            content: "test-app".to_string(),
        }
    }
}

/// アプリケーションのタイトルを表示するための構造体
struct TitleView;

impl iced::application::Title<State> for TitleView {
    fn title(&self, state: &State) -> String {
        // 現在のコンテンツをもとに表示
        state.content.clone()
    }
}

#[derive(Debug, Clone)]
enum Message {
    Increment,
    Decrement,
    SliderChanged(i32),
    ContentChanged(String),
}

impl State {
    fn update(&mut self, message: Message) {
        match message {
            Message::Increment => {
                self.value += 1;
            }
            Message::Decrement => {
                self.value -= 1;
            }
            Message::SliderChanged(value) => {
                self.slider = value;
            }
            Message::ContentChanged(content) => {
                self.content = content;
            }
        }
    }

    fn view(&self) -> Column<Message> {
        column![
            button("Increment").on_press(Message::Increment),
            text(self.value).size(50),
            button("Decrement").on_press(Message::Decrement),
            text_input("Type something here...", &self.content)
                .on_input(Message::ContentChanged)
                .padding(10)
                .size(14),
            slider(-100..=100, self.slider, Message::SliderChanged),
            text(self.slider).size(14),
        ]
        .padding(20)
        .align_x(Center)
    }
}
