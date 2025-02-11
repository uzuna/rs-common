use tracing::debug;

mod window;

fn main() {
    let builder = window::WindowBuilder::new("eye-bin", 1024, 768);

    let mut window = builder.build().expect("Failed to create window");
    let fps = 60;
    let dur = std::time::Duration::from_millis(1000 / fps);
    let mut next = dur;
    'outer: loop {
        // Windowは常にイベントを処理しなけれ場フレームを描画できないので常に回す
        while let Some(event) = window.read_event() {
            if matches!(event, crate::window::WindowEventMsg::DeleteWindow) {
                break 'outer;
            }
            debug!("Event: {:?}", event);
        }
        // wait
        std::thread::sleep(next);
        next += dur;
    }
}
