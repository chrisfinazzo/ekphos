use super::*;

impl App {
    pub fn item_is_diagram_at(&self, index: usize) -> bool {
        matches!(self.document.content_items.get(index), Some(ContentItem::Diagram { .. }))
    }

    pub fn diagram_item_indices(&self) -> Vec<usize> {
        self.document.content_items.iter().enumerate().filter_map(|(index, item)| matches!(item, ContentItem::Diagram { .. }).then_some(index)).collect()
    }

    pub fn open_current_diagram(&mut self) -> bool {
        self.open_diagram_viewer(self.document.content_cursor)
    }

    pub fn open_diagram_viewer(&mut self, item_index: usize) -> bool {
        let Some(ContentItem::Diagram { range, .. }) = self.document.content_items.get(item_index) else {
            return false;
        };
        if self.images.picker.is_none() {
            self.show_toast("This terminal can't display images, so the diagram viewer is unavailable", ToastKind::Error);
            return true;
        }
        let source = self.document_slice(*range).to_string();
        let style = self.state.diagram_viewer.as_ref().map_or(0, |viewer| viewer.style);
        let mut viewer = DiagramViewerState::new(item_index, source);
        viewer.style = style;
        self.document.content_cursor = item_index;
        self.state.diagram_viewer = Some(Box::new(viewer));
        self.state.dialog = DialogState::DiagramViewer;
        true
    }

    pub fn close_diagram_viewer(&mut self) {
        self.state.diagram_viewer = None;
        if self.state.dialog == DialogState::DiagramViewer {
            self.state.dialog = DialogState::None;
        }
        self.state.needs_full_clear = true;
    }

    pub fn step_diagram_viewer(&mut self, step: isize) {
        let Some(current) = self.state.diagram_viewer.as_ref().map(|viewer| viewer.item_index) else {
            return;
        };
        let diagrams = self.diagram_item_indices();
        if diagrams.len() < 2 {
            self.state.status_message = Some("This note has only one diagram".to_string());
            return;
        }
        let position = diagrams.iter().position(|index| *index == current).unwrap_or(0) as isize;
        let next = diagrams[(position + step).rem_euclid(diagrams.len() as isize) as usize];
        self.open_diagram_viewer(next);
    }

    pub fn diagram_position(&self, item_index: usize) -> Option<(usize, usize)> {
        let diagrams = self.diagram_item_indices();
        Some((diagrams.iter().position(|index| *index == item_index)? + 1, diagrams.len()))
    }

    pub fn copy_diagram_source(&mut self) {
        let Some(source) = self.state.diagram_viewer.as_ref().map(|viewer| viewer.source.clone()) else {
            return;
        };
        match self.clipboard().set_text(&source) {
            Ok(()) => self.show_toast("Copied the Mermaid source", ToastKind::Success),
            Err(error) => self.show_error_toast(format!("Couldn't copy the diagram: {error}")),
        }
    }

    pub fn edit_diagram_source(&mut self) {
        let Some(item_index) = self.state.diagram_viewer.as_ref().map(|viewer| viewer.item_index) else {
            return;
        };
        self.close_diagram_viewer();
        if self.item_is_diagram_at(item_index) {
            self.document.content_cursor = item_index;
        }
        self.push_navigation_history(self.vault.selected_note);
        self.enter_edit_mode();
    }
}
