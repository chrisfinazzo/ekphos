use super::*;
use crate::diagram::{diagram_key, diagram_kind, DiagramPalette, DiagramStyle, DiagramView, SVG_UNITS_PER_ROW};
use ratatui::style::Color;
use ratatui::widgets::BorderType;
use ratatui_image::picker::{Picker, ProtocolType};

const SIZE_TOLERANCE: f32 = 0.15;
const CARD_LEFT_INSET: u16 = 2;
const CARD_RIGHT_INSET: u16 = 1;
const PENDING_HEIGHT: u16 = 3;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DiagramBlockState {
    Ready { image_key: String, size: Size },
    Pending,
    Failed { message: String },
    Unsupported,
}

impl DiagramBlockState {
    pub(super) fn height(&self, source_lines: u16, card_width: u16) -> u16 {
        match self {
            Self::Ready { size, .. } => size.height.saturating_add(2),
            Self::Pending => PENDING_HEIGHT,
            Self::Failed { message } => source_lines.saturating_add(2).saturating_add(u16::try_from(failure_lines(message, card_width, Color::Reset).len()).unwrap_or(u16::MAX)),
            Self::Unsupported => source_lines.saturating_add(2),
        }
    }
}

fn failure_lines(message: &str, card_width: u16, error: Color) -> Vec<Line<'static>> {
    let spans = vec![Span::raw(format!("⚠ Couldn't render this diagram: {message}"))];
    distribute_spans_across_lines(spans, usize::from(card_width.saturating_sub(4).max(1)), Color::Reset).into_iter().map(|line| Line::from(line).style(Style::default().fg(error).add_modifier(Modifier::BOLD))).collect()
}

pub(super) fn diagram_card_width(area_width: u16) -> u16 {
    area_width.saturating_sub(CARD_LEFT_INSET + CARD_RIGHT_INSET)
}

pub(crate) fn opaque_rgb(color: Color) -> Option<[u8; 3]> {
    (color != Color::Reset).then(|| terminal_color_rgb(color, Color::Reset))
}

pub(crate) fn diagram_background(theme: &Theme) -> [u8; 3] {
    opaque_rgb(theme.content.background).or_else(|| opaque_rgb(theme.background)).unwrap_or([24, 24, 27])
}

pub(crate) fn diagram_palette(theme: &Theme) -> DiagramPalette {
    let text = opaque_rgb(theme.content.text).or_else(|| opaque_rgb(theme.foreground)).unwrap_or([220, 220, 220]);
    let color = |color: Color| opaque_rgb(color).unwrap_or(text);
    DiagramPalette {
        text,
        muted: color(theme.muted),
        background: diagram_background(theme),
        accent: color(theme.primary),
        secondary: color(theme.secondary),
        info: color(theme.info),
        success: color(theme.success),
        warning: color(theme.warning),
        error: color(theme.error),
        border: color(theme.border),
    }
}

pub(crate) fn diagram_needs_backdrop(picker: &Picker) -> bool {
    matches!(picker.protocol_type(), ProtocolType::Halfblocks | ProtocolType::Sixel)
}

pub(super) fn inline_diagram_size(width: f32, height: f32, font_size: ratatui_image::FontSize, limits: Size) -> Size {
    let cell_width = f32::from(font_size.width.max(1));
    let cell_height = f32::from(font_size.height.max(1));
    let width = width.max(1.0);
    let height = height.max(1.0);
    let scale = (cell_height / SVG_UNITS_PER_ROW).min(f32::from(limits.width.max(1)) * cell_width / width).min(f32::from(limits.height.max(1)) * cell_height / height);
    let cells = |extent: f32, cell: f32, limit: u16| ((extent * scale / cell - SIZE_TOLERANCE).ceil().max(1.0) as u16).min(limit.max(1));
    Size::new(cells(width, cell_width, limits.width), cells(height, cell_height, limits.height))
}

fn source_line_count(source: &str) -> u16 {
    u16::try_from(source.lines().count().max(1)).unwrap_or(u16::MAX)
}

pub(super) fn diagram_block_height(state: Option<&DiagramBlockState>, source: &str, card_width: u16) -> u16 {
    state.map_or(PENDING_HEIGHT, |state| state.height(source_line_count(source), card_width))
}

pub(super) fn prepare_diagrams(app: &mut App, viewport: Size, render_images: bool) -> Vec<Option<DiagramBlockState>> {
    let mut states = vec![None; app.document.content_items.len()];
    let Some(document) = app.document() else {
        return states;
    };
    let diagrams: Vec<(usize, String)> = app
        .document
        .content_items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ContentItem::Diagram { range, .. } => Some((index, document.slice(*range).to_string())),
            _ => None,
        })
        .collect();
    if diagrams.is_empty() {
        return states;
    }
    let font_size = render_images.then(|| app.images.picker.as_ref().map(Picker::font_size)).flatten();
    let style = DiagramStyle::Themed(diagram_palette(&app.state.theme));
    let limits = Size::new(viewport.width.saturating_sub(CARD_LEFT_INSET + CARD_RIGHT_INSET + 2).max(1), app.state.config.effective_diagram_height().min(viewport.height.saturating_sub(2)).max(1));
    for (index, source) in diagrams {
        let Some(font_size) = font_size else {
            states[index] = Some(DiagramBlockState::Unsupported);
            continue;
        };
        let image_key = diagram_key(&source, &style);
        states[index] = Some(if let Some((width, height)) = app.diagram_dimensions(&image_key) {
            DiagramBlockState::Ready { size: inline_diagram_size(width, height, font_size, limits), image_key }
        } else if let Some(message) = app.image_failure(&image_key) {
            DiagramBlockState::Failed { message: message.to_string() }
        } else {
            if !app.is_image_pending(&image_key) {
                app.request_diagram(&image_key, source, style);
            }
            DiagramBlockState::Pending
        });
    }
    states
}

fn diagram_state_key(item_index: usize, image_key: &str) -> String {
    format!("diagram:block:{item_index}:{image_key}")
}

fn ensure_diagram_image_state(app: &mut App, item_index: usize, image_key: &str, size: Size) -> String {
    let state_key = diagram_state_key(item_index, image_key);
    if app.touch_image_state(&state_key, size) || size.width == 0 || size.height == 0 {
        return state_key;
    }
    app.remove_image_state(&state_key);
    let Some(scene) = app.diagram_scene(image_key) else {
        app.reload_image(image_key);
        return state_key;
    };
    let background = diagram_background(&app.state.theme);
    let Some(picker) = app.images.picker.as_ref() else {
        return state_key;
    };
    let font_size = picker.font_size();
    let pixel_width = u32::from(size.width) * u32::from(font_size.width);
    let pixel_height = u32::from(size.height) * u32::from(font_size.height);
    let fill = diagram_needs_backdrop(picker).then_some(background);
    let Ok(image) = scene.rasterize(DiagramView::fit(scene.width(), scene.height(), pixel_width, pixel_height), fill) else {
        return state_key;
    };
    let protocol_bytes = usize::try_from(u64::from(pixel_width) * u64::from(pixel_height) * 4).unwrap_or(usize::MAX);
    if let Ok(protocol) = SlicedProtocol::new_with_resize(picker, image, size, Resize::Fit(None)) {
        app.insert_image_state(state_key.clone(), protocol, size, protocol_bytes);
    }
    state_key
}

pub(super) struct DiagramBlockView<'a> {
    pub(super) item_index: usize,
    pub(super) source: &'a str,
    pub(super) state: &'a DiagramBlockState,
    pub(super) viewport: Rect,
    pub(super) is_cursor: bool,
    pub(super) is_hovered: bool,
}

pub(super) fn render_diagram_block(f: &mut Frame, app: &mut App, view: DiagramBlockView<'_>, area: Rect) {
    let theme = &app.state.theme;
    let (border, warning, info, muted, secondary, error, code) = (theme.border, theme.warning, theme.info, theme.muted, theme.secondary, theme.error, theme.content.code);
    let card_width = diagram_card_width(area.width);
    let full_height = diagram_block_height(Some(view.state), view.source, card_width);
    let card = Rect { x: area.x.saturating_add(CARD_LEFT_INSET), width: card_width, height: full_height, ..area };
    let visible = card.intersection(view.viewport).intersection(area);
    if visible.width < 3 || visible.height == 0 {
        return;
    }
    if view.is_cursor {
        f.render_widget(Paragraph::new(Span::styled("▶", Style::default().fg(warning))), Rect { width: 1, height: 1, ..area });
    }
    let border_color = if view.is_cursor {
        warning
    } else if view.is_hovered {
        info
    } else {
        border
    };
    let mut borders = Borders::LEFT | Borders::RIGHT | Borders::TOP;
    if visible.height >= full_height {
        borders |= Borders::BOTTOM;
    }
    let title = Line::from(vec![Span::styled(" ◇ ", Style::default().fg(secondary)), Span::styled(format!("{} ", diagram_kind(view.source)), Style::default().fg(muted))]);
    let mut block = Block::default().borders(borders).border_type(BorderType::Rounded).border_style(Style::default().fg(border_color)).title(title);
    if (view.is_cursor || view.is_hovered) && matches!(view.state, DiagramBlockState::Ready { .. }) {
        block = block.title(Line::from(Span::styled(" Enter to explore ", Style::default().fg(border_color))).right_aligned());
    }
    let inner = block.inner(visible);
    f.render_widget(block, visible);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let padded = Block::default().padding(ratatui::widgets::Padding::horizontal(1));
    let source_lines = || view.source.lines().map(|line| Line::from(Span::styled(expand_tabs(line), Style::default().fg(code)))).collect::<Vec<_>>();
    match view.state {
        DiagramBlockState::Ready { image_key, size } => {
            let image_area = Rect { x: inner.x.saturating_add(inner.width.saturating_sub(size.width) / 2), y: inner.y, width: size.width.min(inner.width), height: size.height }.intersection(inner);
            if image_area.width > 0 && image_area.height > 0 {
                let state_key = ensure_diagram_image_state(app, view.item_index, image_key, *size);
                if let Some(image_state) = app.images.image_states.get(&state_key) {
                    f.render_widget(SlicedImage::new(&image_state.image, SignedPosition::from((0, 0))), image_area);
                }
            }
        }
        DiagramBlockState::Pending => {
            f.render_widget(Paragraph::new(Span::styled(" Rendering diagram…", Style::default().fg(secondary).add_modifier(Modifier::ITALIC))), inner);
        }
        DiagramBlockState::Failed { message } => {
            let mut lines = failure_lines(message, card_width, error);
            lines.extend(view.source.lines().map(|line| Line::from(Span::styled(expand_tabs(line), Style::default().fg(muted)))));
            f.render_widget(Paragraph::new(lines).block(padded), inner);
        }
        DiagramBlockState::Unsupported => {
            f.render_widget(Paragraph::new(source_lines()).block(padded), inner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: ratatui_image::FontSize = ratatui_image::FontSize { width: 10, height: 20 };

    #[test]
    fn inline_size_uses_terminal_text_scale_when_there_is_room() {
        let size = inline_diagram_size(168.0, 84.0, FONT, Size::new(200, 40));
        assert_eq!(size, Size::new(20, 5));
    }

    #[test]
    fn inline_size_shrinks_to_the_width_and_height_limits() {
        let wide = inline_diagram_size(1680.0, 168.0, FONT, Size::new(50, 40));
        assert_eq!(wide.width, 50);
        assert!(wide.height <= 3);
        let tall = inline_diagram_size(168.0, 1680.0, FONT, Size::new(200, 10));
        assert_eq!(tall.height, 10);
        assert!(tall.width <= 3);
    }

    #[test]
    fn failed_and_unsupported_blocks_reserve_room_for_the_source() {
        assert_eq!(DiagramBlockState::Failed { message: "bad".into() }.height(4, 80), 7);
        assert_eq!(DiagramBlockState::Failed { message: "parse error at line 2: missing closing ']' for node 'A'".into() }.height(4, 42), 9);
        assert_eq!(DiagramBlockState::Unsupported.height(4, 80), 6);
        assert_eq!(DiagramBlockState::Ready { image_key: String::new(), size: Size::new(10, 8) }.height(4, 80), 10);
    }
}
