use iced::widget::{button, column, text, Column};
use iced::{Center, Font};

const ICON_FONT: Font = Font::with_name("aakar");

pub fn main() -> iced::Result {
    let s = iced::settings::Settings {
        default_font: ICON_FONT,
        ..iced::settings::Settings::default()
    };
    iced::application("counter", Counter::update, Counter::view)
        .settings(s)
        .run()
}

#[derive(Default)]
struct Counter {
    value: i64,
}

#[derive(Debug, Clone, Copy)]
enum Message {
    Increment,
    Decrement,
}

impl Counter {
    fn update(&mut self, message: Message) {
        match message {
            Message::Increment => {
                self.value += 1;
            }
            Message::Decrement => {
                self.value -= 1;
            }
        }
    }

    fn view(&self) -> Column<Message> {
        column![
            button("Increment").on_press(Message::Increment),
            text(self.value).size(50),
            button("Decrement").on_press(Message::Decrement)
        ]
        .padding(20)
        .align_x(Center)
    }
}
