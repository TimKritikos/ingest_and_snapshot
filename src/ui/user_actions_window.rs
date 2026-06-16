use ratatui::Frame;
use ratatui::layout::Rect;
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets;

#[derive(PartialEq)]
pub enum SelectedAction {
    Quit,
    ManualTransfer,
    Snapshot,
}

pub struct ActionsWindowState {
    pub selected: SelectedAction,
    /// When true the "Unmount and exit" option is greyed out and unselectable because
    /// transfers are still running. Ignored when the `disable-ui-safety-checks` feature is set.
    pub quit_disabled: bool,
}

impl ActionsWindowState {
    pub fn new() -> Self {
        Self {
            selected: SelectedAction::Quit,
            quit_disabled: false,
        }
    }
}

pub enum ActionsWindowEvent {
    Quit,
    StartManualTransfer,
}

pub fn handle_key(state: &mut ActionsWindowState, key: KeyEvent) -> Option<ActionsWindowEvent> {
    match key.code {
        KeyCode::Up => {
            state.selected = match state.selected {
                SelectedAction::Quit           => SelectedAction::Quit,
                SelectedAction::ManualTransfer => SelectedAction::Quit,
                SelectedAction::Snapshot       => SelectedAction::ManualTransfer,
            };
        }
        KeyCode::Down => {
            state.selected = match state.selected {
                SelectedAction::Quit           => SelectedAction::ManualTransfer,
                SelectedAction::ManualTransfer => SelectedAction::Snapshot,
                SelectedAction::Snapshot       => SelectedAction::Snapshot,
            };
        }
        KeyCode::Enter => {
            return match state.selected {
                SelectedAction::Quit => {
                    let blocked = !cfg!(feature = "disable-ui-safety-checks") && state.quit_disabled;
                    if blocked { None } else { Some(ActionsWindowEvent::Quit) }
                }
                SelectedAction::ManualTransfer => Some(ActionsWindowEvent::StartManualTransfer),
                SelectedAction::Snapshot       => None,
            };
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

    let quit_visually_disabled =
        !cfg!(feature = "disable-ui-safety-checks") && state.quit_disabled;

    let list = tui_dialog_widgets::DialogSelectionList::new(vec![
        "Unmount and exit",
        "Start manual transfer",
        "Finish backup and do snapshot",
    ])
        .title("Options")
        .selected(Some(match state.selected {
            SelectedAction::Quit           => 0,
            SelectedAction::ManualTransfer => 1,
            SelectedAction::Snapshot       => 2,
        }))
        .disabled_indices(if quit_visually_disabled { vec![0] } else { vec![] })
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
