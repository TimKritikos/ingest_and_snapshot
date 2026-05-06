use std::collections::VecDeque;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Paragraph;
use super::tui_dialog_widgets;

pub fn render(frame: &mut Frame, area: Rect, query_queue: &VecDeque<String>) {
    let title = if query_queue.len() > 1 {
        format!("User queries [{} queued]", query_queue.len() - 1)
    } else {
        "User queries".to_string()
    };

    let block = tui_dialog_widgets::DialogBlock::default().title(&title);
    frame.render_widget(block.clone(), area);

    if let Some(front) = query_queue.front() {
        frame.render_widget(Paragraph::new(front.as_str()), block.inner(area));
    }
}
