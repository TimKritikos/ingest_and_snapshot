use ratatui::Frame;
use ratatui::layout::Rect;
use super::tui_dialog_widgets;

pub fn render(frame: &mut Frame, area: Rect) {
    let transfer_window = tui_dialog_widgets::DialogBlock::default()
        .title("Transfers");
    frame.render_widget(transfer_window, area);
}
