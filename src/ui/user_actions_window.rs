use ratatui::Frame;
use ratatui::layout::Rect;
use super::tui_dialog_widgets;

#[derive(PartialEq)]
pub enum SelectedAction {
    Quit,
    Snapshot,
}

pub fn render(frame: &mut Frame, area: Rect, selected_action: &SelectedAction) {
    let actions_window = tui_dialog_widgets::DialogBlock::default()
        .title("Actions");
    frame.render_widget(actions_window.clone(), area);

    let list = tui_dialog_widgets::DialogSelectionList::new(vec![
        "Exit",
        "Finish backup and do snapshot",
    ])
        .title("Options")
        .selected(Some(match selected_action {
            SelectedAction::Quit => 0,
            SelectedAction::Snapshot => 1,
        }))
        .focused(true);

    let actions_window_content = actions_window.inner(area);

    let list_area = Rect {
        x: actions_window_content.x + actions_window_content.width / 2 - 25,
        y: actions_window_content.y + actions_window_content.height / 2 - 3,
        width: 50,
        height: 6,
    };

    frame.render_widget(list, list_area);
}
