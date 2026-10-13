//! Mouse on space group headers: a click toggles a group, and a drag moves
//! its run of spaces. A pinned section's header only toggles.

use super::*;
use crossterm::event::MouseEvent;

/// A press on a space group header.
pub(in crate::client::shell) struct ClientGroupPress {
    endpoint_id: ClientEndpointId,
    key: String,
    member_ids: Vec<String>,
    /// Pressed on a pinned section's header, which only toggles.
    pinned: bool,
    start_column: u16,
    start_row: u16,
}

impl ClientShellState {
    /// Starts a press on the group header under the pointer, if any.
    pub(in crate::client::shell) fn press_group_marker(
        &mut self,
        mouse: MouseEvent,
        point: (u16, u16),
    ) -> bool {
        let group_press = self
            .hits
            .markers
            .iter()
            .find(|hit| super::super::contains(hit.rect, point))
            .and_then(|hit| {
                Some(ClientGroupPress {
                    endpoint_id: hit.endpoint_id.clone(),
                    key: hit.key.clone()?,
                    member_ids: hit.member_ids.clone(),
                    pinned: hit.pinned,
                    start_column: mouse.column,
                    start_row: mouse.row,
                })
            });
        let pressed = group_press.is_some();
        if pressed {
            self.group_press = group_press;
        }
        pressed
    }

    /// Turns a pressed header into a drag of its run once the pointer moves.
    /// Returns whether a header is pressed.
    pub(in crate::client::shell) fn drag_group_press(
        &mut self,
        mouse: MouseEvent,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(press) = self.group_press.as_ref() else {
            return false;
        };
        let delta = mouse
            .column
            .abs_diff(press.start_column)
            .max(mouse.row.abs_diff(press.start_row));
        if delta >= 1 && !press.pinned && press.endpoint_id == self.active_endpoint_id {
            if let Some(target) = self.workspace_drop_target_at(point) {
                self.chrome_drag = Some(ClientChromeDrag::SpaceGroup {
                    member_ids: press.member_ids.clone(),
                    target: Some(target),
                });
                outcome.repaint = true;
            }
        }
        true
    }

    /// Moves a dragged run in front of the drop target.
    pub(in crate::client::shell) fn drop_space_group(
        &mut self,
        member_ids: Vec<String>,
        target: Option<(Option<String>, u16)>,
        outcome: &mut ClientShellInput,
    ) {
        if let Some((before_workspace_id, _)) = target {
            // Dropping a run onto itself changes nothing.
            if !before_workspace_id
                .as_ref()
                .is_some_and(|before| member_ids.contains(before))
            {
                self.push_endpoint_method(
                    crate::api::schema::Method::WorkspaceMoveBlock(
                        crate::api::schema::WorkspaceMoveBlockParams {
                            workspace_ids: member_ids,
                            before_workspace_id,
                        },
                    ),
                    outcome,
                );
            }
        }
        outcome.repaint = true;
    }

    /// Toggles a header pressed without a drag. Returns whether one was.
    pub(in crate::client::shell) fn release_group_press(
        &mut self,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(press) = self.group_press.take() else {
            return false;
        };
        self.toggle_collapsed_group(&press.endpoint_id, press.key);
        outcome.repaint = true;
        self.persist_chrome_preferences(outcome);
        true
    }
}
