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
    /// When true, transfers are still running, so the options that must wait for copying to finish
    /// — "Unmount and exit" (don't stop a transfer midway) and "Finish backup and do snapshot"
    /// (don't snapshot before the copy is done) — are greyed out and unselectable. Ignored when the
    /// `disable-ui-safety-checks` feature is set.
    pub transfers_in_progress: bool,
}

impl ActionsWindowState {
    pub fn new() -> Self {
        Self {
            selected: SelectedAction::Quit,
            transfers_in_progress: false,
        }
    }
}

pub enum ActionsWindowEvent {
    Quit,
    StartManualTransfer,
    StartSnapshot,
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
            let blocked_by_transfers = !cfg!(feature = "disable-ui-safety-checks") && state.transfers_in_progress;
            return match state.selected {
                SelectedAction::Quit     => if blocked_by_transfers { None } else { Some(ActionsWindowEvent::Quit) },
                SelectedAction::Snapshot => if blocked_by_transfers { None } else { Some(ActionsWindowEvent::StartSnapshot) },
                SelectedAction::ManualTransfer => Some(ActionsWindowEvent::StartManualTransfer),
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

    // "Unmount and exit" (index 0) and "Finish backup and do snapshot" (index 2) are both unsafe
    // while transfers are still running.
    let unsafe_actions_disabled =
        !cfg!(feature = "disable-ui-safety-checks") && state.transfers_in_progress;

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
        .disabled_indices(if unsafe_actions_disabled { vec![0, 2] } else { vec![] })
        .focused(focused);

    let actions_window_content = actions_window.inner(area);

    let list_height = 6u16.min(actions_window_content.height);
    let list_width = 50u16.min(actions_window_content.width);
    let list_y = actions_window_content.y + (actions_window_content.height.saturating_sub(list_height)) / 2;
    let list_area = Rect {
        x: actions_window_content.x + actions_window_content.width.saturating_sub(list_width) / 2,
        y: list_y,
        width: list_width,
        height: list_height,
    };

    frame.render_widget(list, list_area);
}
