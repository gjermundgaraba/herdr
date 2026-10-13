//! Draws the agent picker's priority marks, on the rows that
//! `aggregate_navigation/agent_picker.rs` builds.

use super::*;

/// Paints an agent picker row's priority mark, which its label holds right
/// after `prefix`, in `style`, short of the `columns` at the row's end.
pub(super) fn paint_mark(
    b: &mut Buffer,
    rect: Rect,
    prefix: &str,
    columns: u16,
    mark: &str,
    style: Style,
) {
    let x = rect.x.saturating_add(display_width(prefix));
    let end = rect.right().saturating_sub(columns);
    put_text(
        b,
        x,
        rect.y,
        display_width(mark).min(end.saturating_sub(x)),
        mark,
        style,
    );
}
