use ratatui::{
    layout::{Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};
use ratatui_image::{
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
    Resize,
};

use super::content::{diagram_background, diagram_needs_backdrop, diagram_palette, opaque_rgb};
use super::panel::full_view_block;
use crate::app::{App, DiagramViewerState};
use crate::config::Theme;
use crate::diagram::{diagram_key, DiagramStyle};

const HELP: &str = "NAVIGATE\n\n  + / - / scroll     zoom in and out\n  h j k l / arrows   pan\n  H J K L            pan faster\n  drag               pan with the mouse\n  double-click       zoom in at the pointer\n  g / G              jump to the top or bottom\n  f / 0              fit to the screen\n  1                  actual size (100%)\n\nDIAGRAM\n\n  t                  cycle the look: note theme, light, dark\n  [ / ]              previous or next diagram in this note\n  e                  edit the Mermaid source\n  y                  copy the Mermaid source\n\n  ?                  close this help\n  Esc / q            close the viewer";

enum CanvasMessage {
    Rendering,
    Failed(String),
    Unavailable(&'static str),
}

fn viewer_style(index: usize, theme: &Theme) -> DiagramStyle {
    match index {
        0 => DiagramStyle::Themed(diagram_palette(theme)),
        1 => DiagramStyle::Light,
        _ => DiagramStyle::Dark,
    }
}

fn style_label(index: usize) -> &'static str {
    match index {
        0 => "Note theme",
        1 => "Light",
        _ => "Dark",
    }
}

pub fn render_diagram_viewer(frame: &mut Frame, app: &mut App) {
    let Some(mut viewer) = app.state.diagram_viewer.take() else {
        return;
    };
    render(frame, app, &mut viewer);
    app.state.diagram_viewer = Some(viewer);
}

fn render(frame: &mut Frame, app: &mut App, viewer: &mut DiagramViewerState) {
    let area = frame.area();
    let theme = app.state.theme.clone();
    frame.render_widget(Clear, area);
    let block = full_view_block(app.state.config.style, &theme, title(app, viewer, &theme));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 8 || inner.height < 4 {
        return;
    }
    let status_rows = if inner.height >= 8 { 2 } else { 1 };
    let body = Rect { height: inner.height.saturating_sub(status_rows), ..inner };
    let canvas = Rect { width: body.width.saturating_sub(1), height: body.height.saturating_sub(1), ..body };
    viewer.canvas = canvas;
    let message = prepare_frame(app, viewer, &theme);
    match &message {
        None if !viewer.help_visible => {
            if let Some((_, protocol)) = viewer.frame.as_ref() {
                frame.render_widget(SlicedImage::new(protocol, SignedPosition::from((0, 0))), canvas);
            }
            render_scrollbars(frame, viewer, body, &theme);
        }
        None => {}
        Some(CanvasMessage::Rendering) => centered(frame, canvas, vec![Line::from(Span::styled("Rendering diagram…", Style::default().fg(theme.secondary).add_modifier(Modifier::ITALIC)))]),
        Some(CanvasMessage::Failed(error)) => {
            let lines = vec![
                Line::from(Span::styled("⚠ Couldn't render this diagram", Style::default().fg(theme.error).add_modifier(Modifier::BOLD))),
                Line::from(Span::styled(error.clone(), Style::default().fg(theme.dialog.text))),
                Line::default(),
                Line::from(vec![Span::styled("Press ", Style::default().fg(theme.muted)), key("e", &theme), Span::styled(" to edit the source", Style::default().fg(theme.muted))]),
            ];
            centered(frame, canvas, lines);
        }
        Some(CanvasMessage::Unavailable(reason)) => centered(frame, canvas, vec![Line::from(Span::styled(*reason, Style::default().fg(theme.muted)))]),
    }
    render_status(frame, viewer, Rect { y: inner.y + inner.height - status_rows, height: status_rows, ..inner }, &theme, message.is_none());
    if viewer.help_visible {
        render_help(frame, area, &theme);
    }
}

fn title(app: &App, viewer: &DiagramViewerState, theme: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(" DIAGRAM ", Style::default().fg(theme.dialog.title).add_modifier(Modifier::BOLD)), Span::styled(viewer.kind, Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))];
    if let Some((position, total)) = app.diagram_position(viewer.item_index).filter(|(_, total)| *total > 1) {
        spans.push(Span::styled(format!("  {position} of {total}"), Style::default().fg(theme.dialog.text)));
    }
    if let Some(note) = app.current_note() {
        spans.push(Span::styled(format!("  {} ", note.title), Style::default().fg(theme.muted)));
    }
    Line::from(spans)
}

fn prepare_frame(app: &mut App, viewer: &mut DiagramViewerState, theme: &Theme) -> Option<CanvasMessage> {
    let Some(font_size) = app.images.picker.as_ref().map(|picker| picker.font_size()) else {
        return Some(CanvasMessage::Unavailable("This terminal can't display images"));
    };
    viewer.font_size = (font_size.width, font_size.height);
    let style = viewer_style(viewer.style, theme);
    let key = diagram_key(&viewer.source, &style);
    if key != viewer.scene_key {
        viewer.scene_key = key.clone();
        viewer.scene = None;
        viewer.frame = None;
    }
    if viewer.scene.is_none() {
        if let Some(scene) = app.diagram_scene(&key) {
            viewer.scene = Some(scene);
        } else if let Some(error) = app.image_failure(&key) {
            return Some(CanvasMessage::Failed(error.to_string()));
        } else {
            if !app.is_image_pending(&key) {
                app.request_diagram(&key, viewer.source.clone(), style);
            }
            return Some(CanvasMessage::Rendering);
        }
    }
    if viewer.canvas.width == 0 || viewer.canvas.height == 0 {
        return Some(CanvasMessage::Unavailable("The window is too small to show the diagram"));
    }
    if viewer.needs_fit {
        viewer.fit();
    } else {
        let (minimum, maximum) = viewer.zoom_bounds();
        viewer.zoom = viewer.zoom.clamp(minimum, maximum);
        viewer.clamp_center();
    }
    let dialog_background = opaque_rgb(theme.dialog.background).unwrap_or_else(|| diagram_background(theme));
    let picker = app.images.picker.as_ref()?;
    let fill = diagram_needs_backdrop(picker).then_some(dialog_background);
    let frame_key = viewer.frame_key(fill);
    if viewer.frame.as_ref().is_some_and(|(current, _)| *current == frame_key) {
        return None;
    }
    let scene = viewer.scene.clone()?;
    let image = match scene.rasterize(viewer.view(), fill) {
        Ok(image) => image,
        Err(error) => return Some(CanvasMessage::Failed(error)),
    };
    match SlicedProtocol::new_with_resize(picker, image, Size::new(viewer.canvas.width, viewer.canvas.height), Resize::Fit(None)) {
        Ok(protocol) => {
            viewer.frame = Some((frame_key, protocol));
            None
        }
        Err(error) => Some(CanvasMessage::Failed(error.to_string())),
    }
}

fn centered(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX).min(area.height);
    let target = Rect { y: area.y + area.height.saturating_sub(height) / 2, height, ..area };
    frame.render_widget(Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center).wrap(Wrap { trim: true }), target);
}

fn render_scrollbars(frame: &mut Frame, viewer: &DiagramViewerState, body: Rect, theme: &Theme) {
    let Some(((left, right), (top, bottom))) = viewer.visible_fraction() else {
        return;
    };
    let buffer = frame.buffer_mut();
    let track = Style::default().fg(theme.border);
    let thumb = Style::default().fg(theme.primary);
    if top > 0.001 || bottom < 0.999 {
        let x = body.x + body.width - 1;
        let rows = body.height.saturating_sub(1);
        let (start, end) = thumb_span(top, bottom, rows);
        for row in 0..rows {
            let (symbol, style) = if (start..end).contains(&row) { ("┃", thumb) } else { ("│", track) };
            if let Some(cell) = buffer.cell_mut((x, body.y + row)) {
                cell.set_symbol(symbol).set_style(style);
            }
        }
    }
    if left > 0.001 || right < 0.999 {
        let y = body.y + body.height - 1;
        let columns = body.width.saturating_sub(1);
        let (start, end) = thumb_span(left, right, columns);
        for column in 0..columns {
            let (symbol, style) = if (start..end).contains(&column) { ("━", thumb) } else { ("─", track) };
            if let Some(cell) = buffer.cell_mut((body.x + column, y)) {
                cell.set_symbol(symbol).set_style(style);
            }
        }
    }
}

fn thumb_span(start: f32, end: f32, cells: u16) -> (u16, u16) {
    let total = f32::from(cells);
    let first = (start * total).floor().clamp(0.0, (total - 1.0).max(0.0)) as u16;
    let last = ((end * total).ceil() as u16).clamp(first + 1, cells.max(first + 1));
    (first, last)
}

fn key(text: &'static str, theme: &Theme) -> Span<'static> {
    Span::styled(text, Style::default().fg(theme.warning).add_modifier(Modifier::BOLD))
}

fn hint(text: &'static str, theme: &Theme) -> Span<'static> {
    Span::styled(text, Style::default().fg(theme.muted))
}

fn render_status(frame: &mut Frame, viewer: &DiagramViewerState, area: Rect, theme: &Theme, ready: bool) {
    let mut status = vec![Span::styled(format!(" {:.0}%", viewer.zoom * 100.0), Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)), Span::styled(format!("  ·  {}", style_label(viewer.style)), Style::default().fg(theme.dialog.text))];
    if let (true, Some((width, height))) = (ready, viewer.scene_size()) {
        status.push(Span::styled(format!("  ·  {width:.0} × {height:.0} px"), Style::default().fg(theme.muted)));
    }
    frame.render_widget(Paragraph::new(Line::from(status)), Rect { height: 1, ..area });
    if area.height < 2 {
        return;
    }
    let hints = Line::from(vec![
        key(" +/-", theme),
        hint(" zoom  ", theme),
        key("hjkl", theme),
        hint(" pan  ", theme),
        key("f", theme),
        hint(" fit  ", theme),
        key("1", theme),
        hint(" 100%  ", theme),
        key("t", theme),
        hint(" look  ", theme),
        key("[/]", theme),
        hint(" prev/next  ", theme),
        key("e", theme),
        hint(" edit  ", theme),
        key("y", theme),
        hint(" copy  ", theme),
        key("?", theme),
        hint(" help  ", theme),
        key("Esc", theme),
        hint(" close", theme),
    ]);
    frame.render_widget(Paragraph::new(hints), Rect { y: area.y + 1, height: 1, ..area });
}

fn render_help(frame: &mut Frame, area: Rect, theme: &Theme) {
    let width = area.width.saturating_sub(2).min(HELP.lines().map(str::len).max().unwrap_or(0) as u16 + 6);
    let height = area.height.saturating_sub(2).min(HELP.lines().count() as u16 + 4);
    if width < 4 || height < 4 {
        return;
    }
    let popup = Rect::new(area.x + area.width.saturating_sub(width) / 2, area.y + area.height.saturating_sub(height) / 2, width, height);
    frame.render_widget(Clear, popup);
    let block = Block::default().title(" Diagram controls ").borders(Borders::ALL).border_type(BorderType::Rounded).border_style(Style::default().fg(theme.primary)).padding(Padding::uniform(1)).style(Style::default().bg(theme.dialog.background));
    frame.render_widget(Paragraph::new(HELP).block(block).wrap(Wrap { trim: false }).style(Style::default().fg(theme.dialog.text)), popup);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbs_cover_the_visible_fraction_and_stay_in_bounds() {
        assert_eq!(thumb_span(0.0, 0.5, 10), (0, 5));
        assert_eq!(thumb_span(0.5, 1.0, 10), (5, 10));
        assert_eq!(thumb_span(0.99, 1.0, 10), (9, 10));
        assert_eq!(thumb_span(0.0, 0.01, 10), (0, 1));
    }
}
