use super::*;

const ZOOM_STEP: f32 = 1.25;
const WHEEL_ZOOM_STEP: f32 = 1.15;
const DOUBLE_CLICK_ZOOM: f32 = 2.0;
const PAN_STEP: f32 = 0.15;
const FAST_PAN_STEP: f32 = 0.5;
const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

pub(super) fn handle_diagram_viewer_key(app: &mut App, key: crossterm::event::KeyEvent) {
    let Some(viewer) = app.state.diagram_viewer.as_deref_mut() else {
        app.close_diagram_viewer();
        return;
    };
    if viewer.help_visible {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            viewer.help_visible = false;
        }
        return;
    }
    let step = if key.modifiers.contains(KeyModifiers::SHIFT) { FAST_PAN_STEP } else { PAN_STEP };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.close_diagram_viewer(),
        KeyCode::Char('?') => viewer.help_visible = true,
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('i') => viewer.set_zoom(viewer.zoom * ZOOM_STEP, None),
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('o') => viewer.set_zoom(viewer.zoom / ZOOM_STEP, None),
        KeyCode::Char('f') | KeyCode::Char('0') => viewer.fit(),
        KeyCode::Char('1') => viewer.set_zoom(1.0, None),
        KeyCode::Char('h') | KeyCode::Left => viewer.pan_view_fraction(-step, 0.0),
        KeyCode::Char('l') | KeyCode::Right => viewer.pan_view_fraction(step, 0.0),
        KeyCode::Char('k') | KeyCode::Up => viewer.pan_view_fraction(0.0, -step),
        KeyCode::Char('j') | KeyCode::Down => viewer.pan_view_fraction(0.0, step),
        KeyCode::Char('H') => viewer.pan_view_fraction(-FAST_PAN_STEP, 0.0),
        KeyCode::Char('L') => viewer.pan_view_fraction(FAST_PAN_STEP, 0.0),
        KeyCode::Char('K') => viewer.pan_view_fraction(0.0, -FAST_PAN_STEP),
        KeyCode::Char('J') => viewer.pan_view_fraction(0.0, FAST_PAN_STEP),
        KeyCode::Char('g') => {
            viewer.center.1 = f32::NEG_INFINITY;
            viewer.clamp_center();
        }
        KeyCode::Char('G') => {
            viewer.center.1 = f32::INFINITY;
            viewer.clamp_center();
        }
        KeyCode::Char('t') => viewer.style = (viewer.style + 1) % crate::app::DiagramViewerState::STYLES,
        KeyCode::Char(']') | KeyCode::Char('n') | KeyCode::Tab => app.step_diagram_viewer(1),
        KeyCode::Char('[') | KeyCode::Char('N') | KeyCode::BackTab => app.step_diagram_viewer(-1),
        KeyCode::Char('e') => {
            app.edit_diagram_source();
            update_cursor_style(app);
        }
        KeyCode::Char('y') => app.copy_diagram_source(),
        _ => {}
    }
}

pub(super) fn handle_diagram_viewer_mouse(app: &mut App, mouse: crossterm::event::MouseEvent) {
    let Some(viewer) = app.state.diagram_viewer.as_deref_mut() else {
        return;
    };
    if viewer.help_visible {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            viewer.help_visible = false;
        }
        return;
    }
    let canvas = viewer.canvas;
    let inside = canvas.contains(ratatui::layout::Position::new(mouse.column, mouse.row));
    let (cell_width, cell_height) = (f32::from(viewer.font_size.0.max(1)), f32::from(viewer.font_size.1.max(1)));
    let anchor = (f32::from(mouse.column.saturating_sub(canvas.x)) * cell_width + cell_width / 2.0, f32::from(mouse.row.saturating_sub(canvas.y)) * cell_height + cell_height / 2.0);
    match mouse.kind {
        MouseEventKind::ScrollUp if inside => viewer.set_zoom(viewer.zoom * WHEEL_ZOOM_STEP, Some(anchor)),
        MouseEventKind::ScrollDown if inside => viewer.set_zoom(viewer.zoom / WHEEL_ZOOM_STEP, Some(anchor)),
        MouseEventKind::ScrollLeft => viewer.pan_pixels(-cell_width * 4.0, 0.0),
        MouseEventKind::ScrollRight => viewer.pan_pixels(cell_width * 4.0, 0.0),
        MouseEventKind::Down(MouseButton::Left) if inside => {
            let now = std::time::Instant::now();
            let double_click = viewer.last_click.is_some_and(|(when, column, row)| now.duration_since(when) < DOUBLE_CLICK_WINDOW && column.abs_diff(mouse.column) <= 1 && row.abs_diff(mouse.row) <= 1);
            if double_click {
                viewer.last_click = None;
                viewer.drag_origin = None;
                viewer.set_zoom(viewer.zoom * DOUBLE_CLICK_ZOOM, Some(anchor));
            } else {
                viewer.last_click = Some((now, mouse.column, mouse.row));
                viewer.drag_origin = Some((mouse.column, mouse.row));
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((column, row)) = viewer.drag_origin {
                let dx = (f32::from(column) - f32::from(mouse.column)) * cell_width;
                let dy = (f32::from(row) - f32::from(mouse.row)) * cell_height;
                viewer.pan_pixels(dx, dy);
                viewer.drag_origin = Some((mouse.column, mouse.row));
            }
        }
        MouseEventKind::Up(MouseButton::Left) => viewer.drag_origin = None,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppDependencies, ContentItem};
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct DiagramApp {
        app: App,
        root: PathBuf,
        terminal: Terminal<TestBackend>,
    }

    impl DiagramApp {
        fn new() -> Self {
            let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!("ekphos-diagram-viewer-{}-{id}", std::process::id()));
            let vault = root.join("vault");
            std::fs::create_dir_all(&vault).unwrap();
            let note = vault.join("Diagrams.md");
            std::fs::write(&note, "# Diagrams\n\n```mermaid\nflowchart TD\n  A[Start] --> B{Ok?}\n  B --> C[One]\n  B --> D[Two]\n  C --> E[End]\n  D --> E\n```\n\n```mermaid\npie\n  \"a\": 1\n  \"b\": 2\n```\n").unwrap();
            let config = Config { general: crate::config::GeneralConfig { welcome_shown: false, check_updates: false, ..Default::default() }, ..Default::default() };
            let mut app = App::new_injected(config, vault, None, AppDependencies::headless(root.join("config"), root.join("cache")));
            app.state.show_welcome = false;
            app.state.show_changelog = false;
            app.state.dialog = DialogState::None;
            assert!(app.select_note_by_path(&note));
            app.images.picker = Some(ratatui_image::picker::Picker::halfblocks());
            app.state.focus = Focus::Content;
            let mut fixture = Self { app, root, terminal: Terminal::new(TestBackend::new(120, 40)).unwrap() };
            fixture.settle();
            fixture
        }

        fn settle(&mut self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                self.terminal.draw(|frame| crate::ui::render(frame, &mut self.app)).unwrap();
                if !self.app.image_has_background_work() || std::time::Instant::now() > deadline {
                    break;
                }
                while self.app.image_has_background_work() && std::time::Instant::now() < deadline {
                    self.app.poll_pending_images();
                    std::thread::yield_now();
                }
            }
            self.terminal.draw(|frame| crate::ui::render(frame, &mut self.app)).unwrap();
        }

        fn open_first(&mut self) {
            self.app.document.content_cursor = self.app.document.content_items.iter().position(|item| matches!(item, ContentItem::Diagram { .. })).unwrap();
            execute_app_command(&mut self.app, AppCommand::Activate);
            assert_eq!(self.app.state.dialog, DialogState::DiagramViewer);
            self.settle();
        }

        fn press(&mut self, code: KeyCode) {
            let modifiers = if matches!(code, KeyCode::Char(character) if character.is_ascii_uppercase()) { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
            handle_key_event(&mut self.app, crossterm::event::KeyEvent::new(code, modifiers)).unwrap();
            self.terminal.draw(|frame| crate::ui::render(frame, &mut self.app)).unwrap();
        }

        fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
            handle_mouse_event(&mut self.app, crossterm::event::MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE });
        }

        fn viewer(&self) -> &crate::app::DiagramViewerState {
            self.app.state.diagram_viewer.as_deref().unwrap()
        }

        fn scene_point(&self, column: u16, row: u16) -> (f32, f32) {
            let viewer = self.viewer();
            let (width, height) = viewer.canvas_pixels();
            let anchor_x = f32::from(column - viewer.canvas.x) * f32::from(viewer.font_size.0) + f32::from(viewer.font_size.0) / 2.0;
            let anchor_y = f32::from(row - viewer.canvas.y) * f32::from(viewer.font_size.1) + f32::from(viewer.font_size.1) / 2.0;
            (viewer.center.0 + (anchor_x - width / 2.0) / viewer.scale(), viewer.center.1 + (anchor_y - height / 2.0) / viewer.scale())
        }
    }

    impl Drop for DiagramApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn keyboard_controls_zoom_pan_fit_and_close() {
        let mut fixture = DiagramApp::new();
        fixture.open_first();
        let fit = fixture.viewer().zoom;
        assert!((fit - fixture.viewer().fit_zoom()).abs() < 1e-4);

        fixture.press(KeyCode::Char('+'));
        assert!((fixture.viewer().zoom - fit * ZOOM_STEP).abs() < 1e-4);
        fixture.press(KeyCode::Char('+'));
        fixture.press(KeyCode::Char('+'));
        let before = fixture.viewer().center;
        fixture.press(KeyCode::Char('j'));
        assert!(fixture.viewer().center.1 > before.1);
        fixture.press(KeyCode::Char('G'));
        let bottom = fixture.viewer().center.1;
        fixture.press(KeyCode::Char('J'));
        assert_eq!(fixture.viewer().center.1, bottom);
        fixture.press(KeyCode::Char('g'));
        assert!(fixture.viewer().center.1 < bottom);

        fixture.press(KeyCode::Char('1'));
        assert!((fixture.viewer().zoom - 1.0_f32.clamp(fixture.viewer().zoom_bounds().0, fixture.viewer().zoom_bounds().1)).abs() < 1e-4);
        fixture.press(KeyCode::Char('f'));
        assert!((fixture.viewer().zoom - fit).abs() < 1e-4);
        for _ in 0..40 {
            fixture.press(KeyCode::Char('-'));
        }
        assert!((fixture.viewer().zoom - fixture.viewer().zoom_bounds().0).abs() < 1e-4);

        fixture.press(KeyCode::Char('?'));
        assert!(fixture.viewer().help_visible);
        fixture.press(KeyCode::Char('+'));
        assert!(fixture.viewer().help_visible);
        fixture.press(KeyCode::Esc);
        assert!(!fixture.viewer().help_visible);
        assert_eq!(fixture.app.state.dialog, DialogState::DiagramViewer);

        fixture.press(KeyCode::Char('t'));
        assert_eq!(fixture.viewer().style, 1);
        fixture.press(KeyCode::Char(']'));
        assert_eq!(fixture.viewer().kind, "Pie chart");
        assert_eq!(fixture.viewer().style, 1);
        fixture.press(KeyCode::Char(']'));
        assert_eq!(fixture.viewer().kind, "Flowchart");
        fixture.press(KeyCode::Char('['));
        assert_eq!(fixture.viewer().kind, "Pie chart");

        fixture.press(KeyCode::Char('q'));
        assert_eq!(fixture.app.state.dialog, DialogState::None);
        assert!(fixture.app.state.diagram_viewer.is_none());
    }

    #[test]
    fn wheel_zoom_keeps_the_point_under_the_pointer_and_drag_pans() {
        let mut fixture = DiagramApp::new();
        fixture.open_first();
        let canvas = fixture.viewer().canvas;
        let (column, row) = (canvas.x + canvas.width / 3, canvas.y + canvas.height / 3);
        let before = fixture.scene_point(column, row);
        fixture.mouse(MouseEventKind::ScrollUp, column, row);
        fixture.mouse(MouseEventKind::ScrollUp, column, row);
        let after = fixture.scene_point(column, row);
        assert!(fixture.viewer().zoom > fixture.viewer().fit_zoom());
        assert!((before.1 - after.1).abs() < 1.0, "{before:?} {after:?}");

        let center = fixture.viewer().center;
        fixture.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        fixture.mouse(MouseEventKind::Drag(MouseButton::Left), column, row - 2);
        fixture.mouse(MouseEventKind::Up(MouseButton::Left), column, row - 2);
        assert!(fixture.viewer().center.1 > center.1);

        std::thread::sleep(DOUBLE_CLICK_WINDOW);
        let zoom = fixture.viewer().zoom;
        fixture.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        fixture.mouse(MouseEventKind::Up(MouseButton::Left), column, row);
        fixture.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
        assert!((fixture.viewer().zoom - (zoom * DOUBLE_CLICK_ZOOM).min(fixture.viewer().zoom_bounds().1)).abs() < 1e-4);
    }

    #[test]
    fn clicking_an_inline_diagram_opens_it_and_edit_returns_to_its_source() {
        let mut fixture = DiagramApp::new();
        let index = fixture.app.document.content_items.iter().position(|item| matches!(item, ContentItem::Diagram { .. })).unwrap();
        let rect = fixture.app.state.content_item_rects.iter().find_map(|(item, rect)| (*item == index).then_some(*rect)).unwrap();
        fixture.mouse(MouseEventKind::Down(MouseButton::Left), rect.x + rect.width / 2, rect.y + 1);
        assert_eq!(fixture.app.state.dialog, DialogState::DiagramViewer);
        fixture.settle();
        fixture.press(KeyCode::Char('e'));
        assert_eq!(fixture.app.state.dialog, DialogState::None);
        assert_eq!(fixture.app.editor.mode, Mode::Edit);
        assert_eq!(fixture.app.editor.cursor().0, 2);
    }
}
