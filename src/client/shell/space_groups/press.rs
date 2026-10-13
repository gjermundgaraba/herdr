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

/// What group headers change in a drop target
/// (`ClientShellState::workspace_drop_target_at`). A header stands for the
/// first space of its run, so the drop line above a run sits above its header
/// instead of on it. Pinned sections take no drops: their headers are left
/// out here, and [`ClientShellState::pinned_drop_floor`] makes the rows above
/// the regular list's first row take none.
pub(in crate::client::shell) struct DropGroups<'a> {
    /// The active endpoint's regular group headers and dividers.
    markers: Vec<&'a MarkerHit>,
}

impl DropGroups<'_> {
    /// Whether a space row has no slot of its own: a pinned mirror, or the
    /// first space under a header, whose slot the header's replaces.
    pub(in crate::client::shell) fn covers(&self, hit: &WorkspaceHit) -> bool {
        hit.mirror
            || self
                .markers
                .iter()
                .any(|marker| marker.rect.y == hit.rect.y.saturating_sub(1))
    }

    /// One slot above each header, before the first space of its run.
    pub(in crate::client::shell) fn slots(
        &self,
    ) -> impl Iterator<Item = (Option<String>, u16)> + '_ {
        self.markers.iter().filter_map(|hit| {
            Some((
                Some(hit.member_ids.first()?.clone()),
                hit.rect.y.saturating_sub(1),
            ))
        })
    }

    /// The row below the last drawn row, which may be a collapsed group's
    /// header rather than the last space.
    pub(in crate::client::shell) fn bottom(&self, last_space_bottom: u16) -> u16 {
        self.markers
            .iter()
            .map(|hit| hit.rect.bottom())
            .fold(last_space_bottom, u16::max)
    }

    /// Orders slots top to bottom, keeping one where a space would get two in
    /// a row.
    pub(in crate::client::shell) fn order(slots: &mut Vec<(Option<String>, u16)>) {
        slots.sort_by_key(|(_, row)| *row);
        slots.dedup_by(|later, earlier| later.0 == earlier.0);
    }
}

impl ClientShellState {
    /// The row above which a drop has no target: the bottom of the active
    /// endpoint's pinned sections, or 0 without any.
    pub(in crate::client::shell) fn pinned_drop_floor(&self) -> u16 {
        self.hits
            .markers
            .iter()
            .filter(|hit| hit.endpoint_id == self.active_endpoint_id && hit.pinned)
            .map(|hit| hit.rect.bottom())
            .chain(
                self.hits
                    .workspaces
                    .iter()
                    .filter(|hit| hit.endpoint_id == self.active_endpoint_id && hit.mirror)
                    .map(|hit| hit.rect.bottom()),
            )
            .max()
            .unwrap_or(0)
    }

    /// The active endpoint's regular group headers, for a drop target.
    pub(in crate::client::shell) fn drop_groups(&self) -> DropGroups<'_> {
        DropGroups {
            markers: self
                .hits
                .markers
                .iter()
                .filter(|hit| hit.endpoint_id == self.active_endpoint_id && !hit.pinned)
                .collect(),
        }
    }

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
        self.fork.group_press = group_press;
        self.fork.group_press.is_some()
    }

    /// Turns a pressed header into a drag of its run once the pointer moves.
    /// Returns whether a header is pressed.
    pub(in crate::client::shell) fn drag_group_press(
        &mut self,
        mouse: MouseEvent,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(press) = self.fork.group_press.as_ref() else {
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
        let Some(press) = self.fork.group_press.take() else {
            return false;
        };
        self.toggle_collapsed_group(&press.endpoint_id, press.key);
        outcome.repaint = true;
        self.persist_chrome_preferences(outcome);
        true
    }
}
