//! Draws the agent picker's priority marks, on the rows that
//! `aggregate_navigation/agent_picker.rs` builds.

use super::*;

/// Paints `row`'s priority mark, if it has one, over its place in `label`,
/// the row's drawn text, short of the `columns` at the row's end. The mark
/// takes `style`, the row's own, when the row is selected or stale, and is
/// yellow otherwise.
pub(super) fn paint_mark(
    b: &mut Buffer,
    rect: Rect,
    columns: u16,
    row: &ClientNavigatorRow,
    label: &str,
    selected: bool,
    style: Style,
    p: &Palette,
) {
    let Some(mark) = row.mark.as_deref() else {
        return;
    };
    // The label ends with the mark, a space, and the row's own label.
    let Some(prefix) = label
        .strip_suffix(row.label.as_str())
        .and_then(|rest| rest.strip_suffix(' '))
        .and_then(|rest| rest.strip_suffix(mark))
    else {
        return;
    };
    let style = if row.stale || selected {
        style
    } else {
        style.fg(p.yellow)
    };
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
