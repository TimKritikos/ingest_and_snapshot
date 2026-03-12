use ratatui::{DefaultTerminal, Frame};
use std::thread::JoinHandle;
use std::thread;

pub fn init() -> JoinHandle<()> {
    color_eyre::install().unwrap();
    thread::spawn(|| {
        ratatui::run(app).unwrap();
    })
}

fn app(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    loop {
        terminal.draw(render)?;
        if crossterm::event::read()?.is_key_press() {
            break Ok(());
        }
    }
}

fn render(frame: &mut Frame) {
    frame.render_widget("hello from ingest and snapshot", frame.area());
}
