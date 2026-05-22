use ratatui::Frame;
use ratatui::layout::Rect;
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets;

#[derive(PartialEq)]
pub enum SelectedAction {
    Quit,
    Snapshot,
}

pub struct ActionsWindowState {
    pub selected: SelectedAction,
}

impl ActionsWindowState {
    pub fn new() -> Self {
        Self { selected: SelectedAction::Quit }
    }
}

pub enum ActionsWindowEvent {
    Quit,
}

pub fn handle_key(state: &mut ActionsWindowState, key: KeyEvent) -> Option<ActionsWindowEvent> {
    match key.code {
        KeyCode::Up => {
            state.selected = match state.selected {
                SelectedAction::Snapshot => SelectedAction::Quit,
                SelectedAction::Quit     => SelectedAction::Quit,
            };
        }
        KeyCode::Down => {
            state.selected = match state.selected {
                SelectedAction::Quit     => SelectedAction::Snapshot,
                SelectedAction::Snapshot => SelectedAction::Snapshot,
            };
        }
        KeyCode::Enter => {
            if matches!(state.selected, SelectedAction::Quit) {
                return Some(ActionsWindowEvent::Quit);
            }
        }
        _ => {}
    }
    None
}

pub fn render(frame: &mut Frame, area: Rect, state: &ActionsWindowState, focused: bool, minimise: bool) {
    let actions_window = tui_dialog_widgets::DialogBlock::default()
        .title("Actions");
    frame.render_widget(actions_window.clone(), area);

    if minimise { return; }

    let list = tui_dialog_widgets::DialogSelectionList::new(vec![
        "Exit",
        "Finish backup and do snapshot",
    ])
        .title("Options")
        .selected(Some(match state.selected {
            SelectedAction::Quit     => 0,
            SelectedAction::Snapshot => 1,
        }))
        .focused(focused);

    let actions_window_content = actions_window.inner(area);

    let list_height = 6u16.min(actions_window_content.height);
    let list_y = actions_window_content.y + (actions_window_content.height.saturating_sub(list_height)) / 2;
    let list_area = Rect {
        x: actions_window_content.x + actions_window_content.width / 2 - 25,
        y: list_y,
        width: 50,
        height: list_height,
    };

    frame.render_widget(list, list_area);
}
