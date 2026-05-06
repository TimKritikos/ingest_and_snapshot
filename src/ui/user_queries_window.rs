use ratatui::Frame;
use ratatui::layout::Rect;
use super::tui_dialog_widgets;

pub fn render(frame: &mut Frame, area: Rect, allow: &[String], ignore: &[String], active_names: &[&str]) {
    let title = if active_names.len() > 1 {
        format!("User queries [{} queued]", active_names.len() - 1)
    } else {
        "User queries".to_string()
    };
    let user_queries_window = tui_dialog_widgets::DialogBlock::default()
        .title(&title);
    frame.render_widget(user_queries_window.clone(), area);
    frame.render_widget(
        format!("> hello from ingest and snapshot. Allow: {:?} Ignore: {:?} New transfer:{}", allow, ignore, active_names[0]),
        user_queries_window.inner(area),
    );
}
