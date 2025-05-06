use iced::widget::{
    button, center, column, container, horizontal_space, row, slider, text, text_input, Column,
};
use iced::{border, system, Alignment, Element, Font, Task, Theme};

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
    screen: Screen,
    value: i64,
    slider: i32,
    content: String,
    system_info: Option<system::Information>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            value: 0,
            slider: 0,
            content: "test-app".to_string(),
            screen: Screen::Welcome,
            system_info: None,
        }
    }
}

/// 表示内容の切り替え
enum Screen {
    Welcome,
    SystemInfo,
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
enum SystemInfoMsg {
    LoadSystemInfo,
    SystemInfoLoaded(Box<system::Information>),
}

impl SystemInfoMsg {
    fn loaded(info: system::Information) -> Message {
        Message::SystemInfo(SystemInfoMsg::SystemInfoLoaded(Box::new(info)))
    }
}

#[derive(Debug, Clone)]
enum Message {
    Welcome,
    SystemInfo(SystemInfoMsg),
    Increment,
    Decrement,
    SliderChanged(i32),
    ContentChanged(String),
}

impl State {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SystemInfo(s) => match s {
                SystemInfoMsg::LoadSystemInfo => {
                    return system::fetch_information().map(SystemInfoMsg::loaded)
                }
                SystemInfoMsg::SystemInfoLoaded(info) => {
                    self.system_info = Some(*info);
                    self.screen = Screen::SystemInfo;
                }
            },
            Message::Welcome => {
                self.screen = Screen::Welcome;
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
            Message::ContentChanged(content) => {
                self.content = content;
            }
        };
        Task::none()
    }

    fn system_info(info: &system::Information) -> Element<Message> {
        use bytesize::ByteSize;

        let content: Element<_> = {
            let system_name = text!(
                "System name: {}",
                info.system_name.as_ref().unwrap_or(&"unknown".to_string())
            );

            let system_kernel = text!(
                "System kernel: {}",
                info.system_kernel
                    .as_ref()
                    .unwrap_or(&"unknown".to_string())
            );

            let system_version = text!(
                "System version: {}",
                info.system_version
                    .as_ref()
                    .unwrap_or(&"unknown".to_string())
            );

            let system_short_version = text!(
                "System short version: {}",
                info.system_short_version
                    .as_ref()
                    .unwrap_or(&"unknown".to_string())
            );

            let cpu_brand = text!("Processor brand: {}", info.cpu_brand);

            let cpu_cores = text!(
                "Processor cores: {}",
                info.cpu_cores
                    .map_or("unknown".to_string(), |cores| cores.to_string())
            );

            let memory_readable = ByteSize::b(info.memory_total).to_string();

            let memory_total = text!(
                "Memory (total): {} bytes ({memory_readable})",
                info.memory_total,
            );

            let memory_text = if let Some(memory_used) = info.memory_used {
                let memory_readable = ByteSize::b(memory_used).to_string();

                format!("{memory_used} bytes ({memory_readable})")
            } else {
                String::from("None")
            };

            let memory_used = text!("Memory (used): {memory_text}");

            let graphics_adapter = text!("Graphics adapter: {}", info.graphics_adapter);

            let graphics_backend = text!("Graphics backend: {}", info.graphics_backend);

            column![
                system_name,
                system_kernel,
                system_version,
                system_short_version,
                cpu_brand,
                cpu_cores,
                memory_total,
                memory_used,
                graphics_adapter,
                graphics_backend,
                button("Back").on_press(Message::Welcome),
            ]
            .spacing(1)
            .into()
        };

        center(content).into()
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
            text_input("Type something here...", &self.content)
                .on_input(Message::ContentChanged)
                .padding(10)
                .size(14),
            slider(-100..=100, self.slider, Message::SliderChanged),
            text(self.slider).size(14)
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
                ]
                .spacing(20)
                .align_x(Alignment::Center);

                content.into()
            }
            Screen::SystemInfo => Self::system_info(self.system_info.as_ref().unwrap()),
        }
    }
}
