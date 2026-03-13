use ratatui::{DefaultTerminal, Frame};
use std::thread::JoinHandle;
use std::thread;
use std::sync::mpsc::Receiver;
use std::path::PathBuf;

#[derive(Debug)]
pub enum UiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    Quit,
}

pub fn init(rx: Receiver<UiMessage>) -> JoinHandle<()> {
    color_eyre::install().unwrap();
    thread::spawn(|| {
        ratatui::run(|terminal| { app(terminal,rx)}).unwrap();
    })
}

fn app(terminal: &mut DefaultTerminal,rx: Receiver<UiMessage>) -> std::io::Result<()> {
    let mut l_allow:Vec<String> = [].to_vec();
    let mut l_ignore:Vec<String> = [].to_vec();
    loop {
        terminal.draw(|frame|{ render(frame, &l_allow, &l_ignore) })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMessage::AddConfig { allow, ignore } => {
                    l_allow=allow;
                    l_ignore=ignore;
                }
                UiMessage::Quit => return Ok(()),
            }
        }
    }
}

fn render(frame: &mut Frame, allow:&Vec<String>, ignore:&Vec<String>) {
    frame.render_widget(format!("hello from ingest and snapshot. Allow: {:?} Ignore: {:?}",allow,ignore), frame.area());
}
