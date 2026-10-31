use super::*;
use appcore::viewkit::{
    draw_command::DrawCommand,
    event::{EventContext, EventResult, ViewEvent},
    platform::PointerButton,
    view::{Constraints, MeasureContext, PaintContext},
};

#[derive(Default)]
pub(super) struct CanvasInteraction {
    pub viewport: Size,
    pub pan: Point,
    pub drag: Option<(Point, Point)>,
}
impl CanvasInteraction {
    pub fn reset(&mut self) {
        self.pan = Point::new(0.0, 0.0);
        self.drag = None;
    }
}

pub(super) struct ImageCanvas(pub ViewerModel);
impl ImageCanvas {
    fn image_rect(&self, bounds: Rect, document: &ViewerDocument) -> Rect {
        let size = document.dimensions(self.0.rotation.get());
        let zoom = self.0.current_zoom(document);
        let width = size.width * zoom;
        let height = size.height * zoom;
        let mut canvas = self.0.canvas.borrow_mut();
        canvas.pan = clamped_pan(canvas.pan, Size::new(width, height), bounds.size);
        Rect::new(
            bounds.origin.x + (bounds.size.width - width) * 0.5 + canvas.pan.x,
            bounds.origin.y + (bounds.size.height - height) * 0.5 + canvas.pan.y,
            width,
            height,
        )
    }
}
pub(super) fn clamped_pan(pan: Point, image: Size, viewport: Size) -> Point {
    let x = ((image.width - viewport.width) * 0.5).max(0.0);
    let y = ((image.height - viewport.height) * 0.5).max(0.0);
    Point::new(pan.x.clamp(-x, x), pan.y.clamp(-y, y))
}
impl View for ImageCanvas {
    fn measure(&self, constraints: Constraints, _: &mut MeasureContext<'_>) -> Size {
        constraints.maximum
    }
    fn paint(&self, bounds: Rect, context: &mut PaintContext<'_>) {
        self.0.canvas.borrow_mut().viewport = bounds.size;
        let Some(document) = self.0.document.get() else {
            return;
        };
        let rect = self.image_rect(bounds, &document);
        context
            .display_list
            .push(DrawCommand::PushClip { rect: bounds });
        match document.preview(self.0.rotation.get()) {
            Ok(PreviewData::Raster(image)) => Image::new(image)
                .content_mode(ImageContentMode::Fit)
                .accessibility_label(document.display_name())
                .paint(rect, context),
            Ok(PreviewData::Vector(svg)) => Svg::new(svg)
                .original_colors()
                .content_mode(SvgContentMode::Fit)
                .accessibility_label(document.display_name())
                .paint(rect, context),
            Err(_) => Text::body("The image could not be displayed.").paint(bounds, context),
        }
        context.display_list.push(DrawCommand::PopClip);
    }
    fn handle_event(
        &self,
        bounds: Rect,
        event: &ViewEvent,
        context: &mut EventContext<'_>,
    ) -> EventResult {
        if self.0.document.get().is_none() {
            return EventResult::Ignored;
        }
        match event {
            ViewEvent::KeyPressed { key, modifiers } => {
                if modifiers.shortcut() && matches!(key, Key::Character('+') | Key::Character('='))
                {
                    self.0.adjust_zoom(1.25);
                    return EventResult::Consumed;
                }
                if !modifiers.shortcut() && !modifiers.alt() {
                    match key {
                        Key::ArrowLeft => self.0.navigate(-1),
                        Key::ArrowRight => self.0.navigate(1),
                        _ => return EventResult::Ignored,
                    }
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            ViewEvent::ArrowLeft => {
                self.0.navigate(-1);
                EventResult::Consumed
            }
            ViewEvent::ArrowRight => {
                self.0.navigate(1);
                EventResult::Consumed
            }
            ViewEvent::PointerPressed {
                position,
                button: PointerButton::Primary,
            } if bounds.contains(*position) => {
                let mut canvas = self.0.canvas.borrow_mut();
                canvas.drag = Some((*position, canvas.pan));
                EventResult::Consumed
            }
            ViewEvent::PointerMoved { position } if self.0.canvas.borrow().drag.is_some() => {
                let (origin, pan) = self.0.canvas.borrow().drag.unwrap();
                self.0.canvas.borrow_mut().pan =
                    Point::new(pan.x + position.x - origin.x, pan.y + position.y - origin.y);
                if let Some(document) = self.0.document.get() {
                    self.image_rect(bounds, &document);
                }
                context.request_redraw_in(bounds);
                EventResult::Consumed
            }
            ViewEvent::PointerReleased {
                button: PointerButton::Primary,
                ..
            } if self.0.canvas.borrow().drag.is_some() => {
                self.0.canvas.borrow_mut().drag = None;
                EventResult::Consumed
            }
            ViewEvent::PointerLeft | ViewEvent::FocusChanged { focused: false } => {
                let mut canvas = self.0.canvas.borrow_mut();
                canvas.drag = None;
                EventResult::Ignored
            }
            ViewEvent::Scroll {
                position,
                delta_x,
                delta_y,
            } if bounds.contains(*position) => {
                if delta_x.abs() > delta_y.abs() {
                    self.0.canvas.borrow_mut().pan.x += delta_x;
                } else {
                    let anchor = Point::new(
                        position.x - bounds.origin.x - bounds.size.width * 0.5,
                        position.y - bounds.origin.y - bounds.size.height * 0.5,
                    );
                    self.0
                        .zoom_at((delta_y * 0.002).clamp(-1.0, 1.0).exp(), anchor);
                }
                if let Some(document) = self.0.document.get() {
                    self.image_rect(bounds, &document);
                }
                context.request_redraw_in(bounds);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appcore::viewkit::{
        draw_command::{DisplayList, DrawCommand},
        typography::TextMeasurer,
    };
    fn model() -> ViewerModel {
        let model = ViewerModel::empty();
        model.set_document(ViewerDocument {
            path: PathBuf::from("/unavailable/test.png"),
            preview: PreviewData::Raster(
                ImageData::from_rgba8(2000, 1000, vec![255; 8_000_000]).unwrap(),
            ),
            width: 2000.0,
            height: 1000.0,
            format: "PNG",
            rotated: Rc::new(RefCell::new(None)),
        });
        model
    }
    #[test]
    fn fit_uses_the_actual_canvas_and_zoom_does_not_jump_to_one_hundred_percent() {
        let model = model();
        let canvas = ImageCanvas(model.clone());
        let bounds = Rect::new(50.0, 80.0, 500.0, 300.0);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut list = DisplayList::new();
        canvas.paint(
            bounds,
            &mut PaintContext::new(&mut list, &theme, &theme.typography, &mut measurer),
        );
        let DrawCommand::DrawImage { command } = list
            .commands()
            .iter()
            .find(|command| matches!(command, DrawCommand::DrawImage { .. }))
            .unwrap()
        else {
            panic!()
        };
        assert!(bounds.contains(command.bounds.origin));
        assert!(
            command.bounds.origin.x + command.bounds.size.width
                <= bounds.origin.x + bounds.size.width
        );
        assert!(
            command.bounds.origin.y + command.bounds.size.height
                <= bounds.origin.y + bounds.size.height
        );
        let before = model.current_zoom(&model.document.get().unwrap());
        assert!(before < 0.5);
        model.adjust_zoom(1.25);
        assert!((model.zoom.get() - before * 1.25).abs() < 0.0001);
    }
    #[test]
    fn zoom_keeps_the_anchored_point_and_pan_stays_inside_the_image_edges() {
        let model = model();
        model.canvas.borrow_mut().viewport = Size::new(500.0, 300.0);
        model.actual_size();
        let anchor = Point::new(100.0, 40.0);
        model.zoom_at(2.0, anchor);
        assert_eq!(model.canvas.borrow().pan, Point::new(-100.0, -40.0));
        let pan = clamped_pan(
            Point::new(9999.0, -9999.0),
            Size::new(1000.0, 800.0),
            Size::new(500.0, 300.0),
        );
        assert_eq!(pan, Point::new(250.0, -250.0));
        assert_eq!(
            clamped_pan(pan, Size::new(100.0, 80.0), Size::new(500.0, 300.0)),
            Point::new(0.0, 0.0)
        );
    }
    #[test]
    fn drag_and_wheel_operate_without_rebuilding_interaction_state() {
        let model = model();
        model.actual_size();
        model.canvas.borrow_mut().viewport = Size::new(500.0, 300.0);
        let canvas = ImageCanvas(model.clone());
        let bounds = Rect::new(0.0, 0.0, 500.0, 300.0);
        let theme = Theme::current();
        let mut measurer = TextMeasurer::new();
        let mut context = EventContext::new(&theme, &theme.typography, &mut measurer);
        canvas.handle_event(
            bounds,
            &ViewEvent::PointerPressed {
                position: Point::new(200.0, 150.0),
                button: PointerButton::Primary,
            },
            &mut context,
        );
        canvas.handle_event(
            bounds,
            &ViewEvent::PointerMoved {
                position: Point::new(250.0, 175.0),
            },
            &mut context,
        );
        assert_eq!(model.canvas.borrow().pan, Point::new(50.0, 25.0));
        canvas.handle_event(
            bounds,
            &ViewEvent::PointerReleased {
                position: Point::new(250.0, 175.0),
                button: PointerButton::Primary,
            },
            &mut context,
        );
        assert!(model.canvas.borrow().drag.is_none());
        canvas.handle_event(
            bounds,
            &ViewEvent::Scroll {
                position: Point::new(250.0, 150.0),
                delta_x: 0.0,
                delta_y: 100.0,
            },
            &mut context,
        );
        assert!(model.zoom.get() > 1.0);
        canvas.handle_event(
            bounds,
            &ViewEvent::FocusChanged { focused: false },
            &mut context,
        );
        assert!(model.canvas.borrow().drag.is_none());
    }
}
