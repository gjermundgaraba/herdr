use super::*;

pub(super) fn render_pick_overlay(
    b: &mut Buffer,
    pick: &ClientPickOverlay,
    p: &Palette,
) -> Option<OverlayRender> {
    let rows = pick.rows();
    let popup_height = (pick.items.len() + 8).clamp(10, 24) as u16;
    let popup = popup(b.area, 64, popup_height)?;
    let inner = panel(b, popup, p.accent, p.panel_bg)?;
    put_text(
        b,
        inner.x,
        inner.y,
        inner.width,
        &pick.title,
        Style::default()
            .fg(p.text)
            .bg(p.panel_bg)
            .add_modifier(Modifier::BOLD),
    );
    let search = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    put_text(
        b,
        search.x,
        search.y,
        search.width,
        if pick.query.is_empty() {
            " / type to filter"
        } else {
            " / "
        },
        Style::default().fg(p.overlay0).bg(p.panel_bg),
    );
    let cursor = text_editor::render(
        b,
        Rect::new(search.x + 3, search.y, search.width.saturating_sub(4), 1),
        &pick.query,
        Style::default().fg(p.text).bg(p.panel_bg),
    );
    put_text(
        b,
        inner.x,
        inner.y + 2,
        inner.width,
        &"─".repeat(inner.width as usize),
        Style::default().fg(p.surface1).bg(p.panel_bg),
    );
    let body = Rect::new(
        inner.x,
        inner.y + 3,
        inner.width,
        inner.height.saturating_sub(5),
    );
    let visible_count = (body.height as usize).max(1);
    let start = pick
        .selected
        .saturating_sub(visible_count.saturating_sub(1))
        .min(rows.len().saturating_sub(visible_count));
    let mut row_hits = Vec::new();
    for (visible, (row_index, row)) in rows
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_count)
        .enumerate()
    {
        let rect = Rect::new(body.x, body.y + visible as u16, body.width, 1);
        row_hits.push((rect, row_index));
        let style = if row_index == pick.selected {
            Style::default().fg(contrast(p)).bg(p.accent)
        } else {
            Style::default().fg(p.text).bg(p.panel_bg)
        };
        b.set_style(rect, style);
        match row {
            ClientPickRow::Item(index) => {
                let item = &pick.items[*index];
                let label = match item.badge.as_deref() {
                    Some(badge) => format!(" {badge} {}", item.label),
                    None => format!(" {}", item.label),
                };
                put_text(b, rect.x, rect.y, rect.width, &label, style);
                if let Some(detail) = item.detail.as_deref() {
                    put_right_text(b, rect, rect.y, &format!("{detail} "), style);
                }
            }
            ClientPickRow::Create => {
                let label = pick
                    .create
                    .as_ref()
                    .map_or("Create", |create| create.label.as_str());
                put_text(
                    b,
                    rect.x,
                    rect.y,
                    rect.width,
                    &format!(" + {label} \"{}\"", pick.query.trim()),
                    style.add_modifier(Modifier::ITALIC),
                );
            }
        }
    }
    if rows.is_empty() {
        put_text(
            b,
            body.x,
            body.y,
            body.width,
            " no matches",
            Style::default().fg(p.overlay0).bg(p.panel_bg),
        );
    }
    let buttons = row(inner, &[11, 12], 2, inner.height.saturating_sub(1));
    let [primary, cancel] = buttons.as_slice() else {
        return None;
    };
    button(
        b,
        *primary,
        " ↵ choose ",
        Style::default()
            .fg(contrast(p))
            .bg(p.accent)
            .add_modifier(Modifier::BOLD),
    );
    button(
        b,
        *cancel,
        " esc cancel ",
        Style::default()
            .fg(p.text)
            .bg(p.surface0)
            .add_modifier(Modifier::BOLD),
    );
    Some(OverlayRender {
        area: popup,
        primary: *primary,
        cancel: *cancel,
        list_search: search,
        list_rows: row_hits,
        cursor,
        ..OverlayRender::default()
    })
}
