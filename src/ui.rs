//! Application chrome is composed from standard ViewKit controls.
use super::*;
use appcore::viewkit::view::{Constraints, MeasureContext, PaintContext};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Open,
    Previous,
    Next,
    Rotate,
    ZoomOut,
    ActualSize,
    Fit,
    ZoomIn,
}

pub(super) fn toolbar(
    model: &ViewerModel,
    panel: &OpenPanel,
    state: ActionToolbarState<Action>,
) -> ActionToolbar<Action> {
    let has_document = model.document.get().is_some();
    let open_model = model.clone();
    let open_panel = panel.clone();
    let previous = model.clone();
    let next = model.clone();
    let rotate = model.clone();
    let zoom_out = model.clone();
    let actual = model.clone();
    let fit = model.clone();
    let zoom_in = model.clone();
    ActionToolbar::new(state)
        .context_key(model.document_revision.get())
        .leading(
            Action::Open,
            Button::new("Open…").on_click(move || open_model.show_open(&open_panel)),
        )
        .leading(
            Action::Previous,
            Button::new("")
                .leading_symbol(SymbolName::ChevronLeft)
                .accessibility_label("Previous Image")
                .enabled(model.neighbor(-1).is_some())
                .on_click(move || previous.navigate(-1)),
        )
        .leading(
            Action::Next,
            Button::new("")
                .leading_symbol(SymbolName::ChevronRight)
                .accessibility_label("Next Image")
                .enabled(model.neighbor(1).is_some())
                .on_click(move || next.navigate(1)),
        )
        .leading(
            Action::Rotate,
            Button::new("Rotate")
                .enabled(has_document)
                .on_click(move || rotate.rotate(true)),
        )
        .trailing(
            Action::ZoomOut,
            Button::new("")
                .leading_symbol(SymbolName::SearchMinus)
                .accessibility_label("Zoom Out")
                .enabled(has_document)
                .on_click(move || zoom_out.adjust_zoom(0.8)),
        )
        .trailing(
            Action::ActualSize,
            Button::new("Actual Size")
                .enabled(has_document)
                .on_click(move || actual.actual_size()),
        )
        .trailing(
            Action::Fit,
            Button::new("Fit")
                .accessibility_label("Fit to Window")
                .enabled(has_document)
                .on_click(move || fit.fit()),
        )
        .trailing(
            Action::ZoomIn,
            Button::new("")
                .leading_symbol(SymbolName::SearchPlus)
                .accessibility_label("Zoom In")
                .enabled(has_document)
                .on_click(move || zoom_in.adjust_zoom(1.25)),
        )
}

/// Rebuild only the text at layout/paint time so Fit reports the actual canvas
/// size after resizing. ViewKit owns text measurement, truncation and placement.
pub(super) struct DocumentStatus(pub ViewerModel);
impl DocumentStatus {
    fn content(&self) -> HStack {
        let Some(document) = self.0.document.get() else {
            return HStack::new().child(Text::metadata("Open an image to begin").single_line(true));
        };
        let turns = self.0.rotation.get();
        let size = document.dimensions(turns);
        let zoom = self.0.current_zoom(&document);
        let position = self.0.siblings.with(|items| {
            items
                .iter()
                .position(|path| path.file_name() == document.path.file_name())
                .map(|index| format!("{} / {}", index + 1, items.len()))
                .unwrap_or_default()
        });
        let status = format!(
            "{} · {} × {} · {:.0}%{}",
            document.format,
            size.width as u32,
            size.height as u32,
            zoom * 100.0,
            if turns == 0 {
                String::new()
            } else {
                format!(" · {}°", u16::from(turns) * 90)
            }
        );
        let detail = if self.0.navigation_error.borrow().is_some() {
            "Folder navigation unavailable".into()
        } else {
            format!("{} · {position}", document.display_name())
        };
        HStack::new()
            .alignment(StackAlignment::Center)
            .gap(StackGap::Medium)
            .child(
                Text::metadata(status)
                    .single_line(true)
                    .layout()
                    .flex_grow(1.0),
            )
            .child(
                Text::metadata(detail)
                    .single_line(true)
                    .alignment(TextAlignment::End)
                    .layout()
                    .flex_grow(1.0),
            )
    }
}
impl View for DocumentStatus {
    fn measure(&self, constraints: Constraints, context: &mut MeasureContext<'_>) -> Size {
        self.content().measure(constraints, context)
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.content().paint(bounds, context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appcore::viewkit::{
        draw_command::{DisplayList, DrawCommand},
        event::{EventContext, ViewEvent},
        platform::PointerButton,
        typography::TextMeasurer,
    };

    fn fixture() -> (ViewerModel, OpenPanel) {
        let model = ViewerModel::empty();
        model.set_document(ViewerDocument {
            path: PathBuf::from("/unavailable/test.png"),
            preview: PreviewData::Raster(
                ImageData::from_rgba8(1000, 500, vec![255; 2_000_000]).unwrap(),
            ),
            width: 1000.0,
            height: 500.0,
            format: "PNG",
            rotated: Rc::new(RefCell::new(None)),
        });
        (
            model,
            OpenPanel::new(OpenPanelOptions::default(), |_| Ok(())),
        )
    }

    #[test]
    fn narrow_toolbar_uses_native_overflow_and_rejects_stale_document_actions() {
        let (model, panel) = fixture();
        let theme = Theme::LIGHT;
        let mut text = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut text);
        let state = ActionToolbarState::new();
        let bounds = Rect::new(0.0, 0.0, 240.0, theme.layout.top_bar_height);
        for stale in [false, true] {
            model.fit();
            let bar = toolbar(&model, &panel, state.clone());
            let frame = bar
                .action_bounds(&Action::ActualSize, bounds, &mut context.measure_context())
                .unwrap();
            let position = Point::new(
                frame.origin.x + frame.size.width / 2.0,
                frame.origin.y + frame.size.height / 2.0,
            );
            for event in [
                ViewEvent::PointerPressed {
                    position,
                    button: PointerButton::Primary,
                },
                ViewEvent::PointerReleased {
                    position,
                    button: PointerButton::Primary,
                },
            ] {
                bar.handle_event(bounds, &event, &mut context);
            }
            let request = context
                .take_context_menu_request()
                .expect("native overflow menu");
            let actual = request
                .items
                .iter()
                .find(|item| item.label == "Actual Size")
                .unwrap();
            assert!(actual.enabled);
            if stale {
                model.set_document(model.document.get().unwrap());
            }
            toolbar(&model, &panel, state.clone()).handle_event(
                bounds,
                &ViewEvent::ContextMenuResult {
                    request_id: request.request_id,
                    command_id: Some(actual.command_id),
                },
                &mut context,
            );
            assert_eq!(model.fit_to_window.get(), stale);
            assert_eq!(model.zoom.get(), 1.0);
        }
    }

    #[test]
    fn status_remeasures_fit_after_resize_and_keeps_labels_on_one_line() {
        let (model, _) = fixture();
        let theme = Theme::LIGHT;
        let mut text = TextMeasurer::new();
        let status = DocumentStatus(model.clone());
        for width in [240.0, 800.0] {
            model.canvas.borrow_mut().viewport = Size::new(width, 400.0);
            let bounds = Rect::new(0.0, 0.0, width, theme.layout.bottom_bar_height);
            let mut list = DisplayList::new();
            Toolbar::bottom(DocumentStatus(model.clone())).paint(
                bounds,
                &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut text),
            );
            let expected = format!(
                "{:.0}%",
                model.current_zoom(&model.document.get().unwrap()) * 100.0
            );
            assert!(list.commands().iter().any(|command| matches!(
                command, DrawCommand::DrawText { command } if command.text.contains(&expected)
            )));
            let size = status.measure(
                Constraints::loose(bounds.size),
                &mut MeasureContext {
                    theme: &theme,
                    typography: &theme.typography,
                    text_measurer: &mut text,
                },
            );
            assert!(size.height <= theme.typography.caption.line_height);
        }
    }
}
