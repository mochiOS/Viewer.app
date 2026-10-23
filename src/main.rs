mod canvas;

use std::cell::RefCell;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use appcore::prelude::*;

const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 8.0;
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
enum PreviewData {
    Raster(ImageData),
    Vector(SvgData),
}

#[derive(Clone)]
struct ViewerDocument {
    path: PathBuf,
    preview: PreviewData,
    width: f32,
    height: f32,
    format: &'static str,
    rotated: Rc<RefCell<Option<(u8, PreviewData)>>>,
}

impl ViewerDocument {
    fn dimensions(&self, turns: u8) -> Size {
        if turns % 2 == 0 {
            Size::new(self.width, self.height)
        } else {
            Size::new(self.height, self.width)
        }
    }
    fn preview(&self, turns: u8) -> Result<PreviewData, String> {
        let turns = turns % 4;
        if turns == 0 {
            return Ok(self.preview.clone());
        }
        if let Some((cached, preview)) = &*self.rotated.borrow()
            && *cached == turns
        {
            return Ok(preview.clone());
        }
        let result = match &self.preview {
            PreviewData::Raster(image) => PreviewData::Raster(
                image
                    .rotated_quarter_turns(turns)
                    .map_err(|e| e.to_string())?,
            ),
            PreviewData::Vector(svg) => PreviewData::Vector(
                svg.rotated_quarter_turns(turns)
                    .map_err(|e| e.to_string())?,
            ),
        };
        *self.rotated.borrow_mut() = Some((turns, result.clone()));
        Ok(result)
    }
    fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Document")
            .to_owned()
    }
}

#[derive(Clone)]
struct ViewerModel {
    document: State<Option<ViewerDocument>>,
    zoom: State<f32>,
    fit_to_window: State<bool>,
    alert: Alert,
    rotation: State<u8>,
    siblings: State<Vec<PathBuf>>,
    canvas: Rc<RefCell<canvas::CanvasInteraction>>,
    navigation_error: Rc<RefCell<Option<String>>>,
}

impl ViewerModel {
    fn new() -> Self {
        let model = Self::empty();
        if let Some(path) = document_argument() {
            model.open_or_alert(path);
        }
        model
    }
    fn empty() -> Self {
        Self {
            document: State::new(None),
            zoom: State::new(1.0),
            fit_to_window: State::new(true),
            alert: Alert::new(),
            rotation: State::new(0),
            siblings: State::new(Vec::new()),
            canvas: Rc::new(RefCell::new(canvas::CanvasInteraction::default())),
            navigation_error: Rc::new(RefCell::new(None)),
        }
    }

    fn open(&self, path: &Path) -> Result<(), String> {
        let document = load_document(path)?;
        self.set_document(document);
        Ok(())
    }

    fn open_delegated(&self, opened: document::OpenedDocument) -> Result<(), String> {
        let path = opened.path.clone();
        let bytes = opened
            .read_to_end_limited(MAX_DOCUMENT_BYTES as usize)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let document = decode_document_bytes(&path, &bytes)?;
        eprintln!("Viewer.app: opened delegated document {}", path.display());
        self.set_document(document);
        Ok(())
    }

    fn set_document(&self, document: ViewerDocument) {
        let (siblings, error) = match sibling_images(&document.path) {
            Ok(paths) => (paths, None),
            Err(error) => (vec![document.path.clone()], Some(error)),
        };
        self.siblings.set(siblings);
        *self.navigation_error.borrow_mut() = error;
        self.rotation.set(0);
        self.canvas.borrow_mut().reset();
        self.document.set(Some(document));
        self.zoom.set(1.0);
        self.fit_to_window.set(true);
    }

    fn open_or_alert(&self, path: PathBuf) {
        if let Err(message) = self.open(&path) {
            self.alert
                .present_error("The document could not be opened.", message);
        }
    }

    fn open_delegated_or_alert(&self, opened: document::OpenedDocument) {
        if let Err(message) = self.open_delegated(opened) {
            self.alert
                .present_error("The document could not be opened.", message);
        }
    }

    fn current_zoom(&self, document: &ViewerDocument) -> f32 {
        let viewport = self.canvas.borrow().viewport;
        zoom_for_viewport(
            document.dimensions(self.rotation.get()),
            viewport,
            self.fit_to_window.get(),
            self.zoom.get(),
        )
    }
    fn adjust_zoom(&self, factor: f32) {
        self.zoom_at(factor, Point::new(0.0, 0.0));
    }
    fn zoom_at(&self, factor: f32, anchor: Point) {
        let Some(document) = self.document.get() else {
            return;
        };
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let previous = self.current_zoom(&document);
        let next = (previous * factor).clamp(MIN_ZOOM.min(previous), MAX_ZOOM);
        {
            let mut canvas = self.canvas.borrow_mut();
            canvas.drag = None;
            canvas.pan = Point::new(
                (canvas.pan.x - anchor.x) * next / previous + anchor.x,
                (canvas.pan.y - anchor.y) * next / previous + anchor.y,
            );
        }
        self.fit_to_window.set(false);
        self.zoom.set(next);
    }
    fn show_open(&self, panel: &OpenPanel) {
        let directory = self
            .document
            .get()
            .and_then(|document| document.path.parent().map(Path::to_path_buf));
        panel.show_at(directory.as_deref());
    }
    fn actual_size(&self) {
        self.canvas.borrow_mut().reset();
        self.fit_to_window.set(false);
        self.zoom.set(1.0);
    }
    fn fit(&self) {
        self.canvas.borrow_mut().reset();
        self.fit_to_window.set(true);
    }
    fn rotate(&self, clockwise: bool) {
        let Some(document) = self.document.get() else {
            return;
        };
        let turns = (self.rotation.get() + if clockwise { 1 } else { 3 }) % 4;
        if let Err(error) = document.preview(turns) {
            self.alert
                .present_error("The image could not be rotated.", error);
            return;
        }
        self.canvas.borrow_mut().reset();
        self.rotation.set(turns);
    }
    fn neighbor(&self, offset: isize) -> Option<PathBuf> {
        let document = self.document.get()?;
        self.siblings
            .with(|items| neighbor_path(items, &document.path, offset))
    }
    fn navigate(&self, offset: isize) {
        if let Some(path) = self.neighbor(offset) {
            self.open_or_alert(path);
        }
    }
}

struct ViewerApp {
    model: ViewerModel,
    open_panel: OpenPanel,
}

impl App for ViewerApp {
    type Body = Box<dyn View + 'static>;

    fn new() -> Self {
        let model = ViewerModel::new();
        let panel_model = model.clone();
        let allowed_content_types = [
            "image/png",
            "image/jpeg",
            "image/webp",
            "image/gif",
            "image/bmp",
            "image/svg+xml",
        ]
        .into_iter()
        .filter_map(|content_type| ContentType::parse(content_type).ok())
        .collect();
        let open_panel = OpenPanel::new(
            OpenPanelOptions {
                title: String::from("Open in Viewer"),
                allowed_content_types,
                ..OpenPanelOptions::default()
            },
            move |path| panel_model.open(path),
        );
        Self { model, open_panel }
    }

    fn window(&self) -> WindowOptions {
        let layout = Theme::current().layout;
        let title = self.model.document.with(|document| {
            document
                .as_ref()
                .map(|document| format!("{} — Viewer", document.display_name()))
                .unwrap_or_else(|| String::from("Viewer"))
        });
        WindowOptions::new(title)
            .size(layout.standard_window_width, layout.standard_window_height)
            .resizable(true)
    }

    fn body(&self, _context: &ViewContext) -> Self::Body {
        let document = self.model.document.get();
        let has_document = document.is_some();
        let panel_for_open = self.open_panel.clone();
        let menu_panel = self.open_panel.clone();
        let toolbar_open_model = self.model.clone();
        let menu_open_model = self.model.clone();
        let fit_model = self.model.clone();
        let actual_model = self.model.clone();
        let zoom_out_model = self.model.clone();
        let zoom_in_model = self.model.clone();

        let previous_model = self.model.clone();
        let next_model = self.model.clone();
        let rotate_model = self.model.clone();
        let toolbar = Toolbar::new(canvas::ViewerToolbar {
            leading: HStack::new()
                .alignment(StackAlignment::Center)
                .gap(StackGap::Small)
                .child(
                    Button::new("Open…")
                        .size(ButtonSize::Small)
                        .on_click(move || toolbar_open_model.show_open(&panel_for_open)),
                )
                .child(
                    IconButton::new(SymbolName::ChevronLeft)
                        .enabled(self.model.neighbor(-1).is_some())
                        .accessibility_label("Previous Image")
                        .on_click(move || previous_model.navigate(-1)),
                )
                .child(
                    IconButton::new(SymbolName::ChevronRight)
                        .enabled(self.model.neighbor(1).is_some())
                        .accessibility_label("Next Image")
                        .on_click(move || next_model.navigate(1)),
                )
                .child(
                    Button::new("Rotate")
                        .size(ButtonSize::Small)
                        .enabled(has_document)
                        .on_click(move || rotate_model.rotate(true)),
                ),
            trailing: HStack::new()
                .alignment(StackAlignment::Center)
                .gap(StackGap::Small)
                .child(
                    IconButton::new(SymbolName::SearchMinus)
                        .enabled(has_document)
                        .on_click(move || zoom_out_model.adjust_zoom(0.8))
                        .accessibility_label("Zoom Out"),
                )
                .child(
                    Button::new("Actual Size")
                        .size(ButtonSize::Small)
                        .enabled(has_document)
                        .on_click(move || actual_model.actual_size()),
                )
                .child(
                    Button::new("Fit")
                        .size(ButtonSize::Small)
                        .enabled(has_document)
                        .on_click(move || fit_model.fit()),
                )
                .child(
                    IconButton::new(SymbolName::SearchPlus)
                        .enabled(has_document)
                        .on_click(move || zoom_in_model.adjust_zoom(1.25))
                        .accessibility_label("Zoom In"),
                ),
        });
        let content: Box<dyn View> = if has_document {
            Box::new(canvas::ImageCanvas(self.model.clone()))
        } else {
            empty_view(self.open_panel.clone())
        };
        let status_bar = Toolbar::bottom(canvas::ViewerStatus(self.model.clone()));

        let drop_model = self.model.clone();
        let main_content = FileDropTarget::new(
            Surface::app().content(
                VStack::new()
                    .alignment(StackAlignment::Stretch)
                    .gap(StackGap::None)
                    .child(toolbar)
                    .child(Divider::new())
                    .child(content.layout().flex_grow(1.0))
                    .child(Divider::new())
                    .child(status_bar),
            ),
            move |path| drop_model.open_or_alert(path),
        )
        .accepts(is_supported_path);

        let root = Overlay::new()
            .content(main_content)
            .overlay(self.open_panel.clone());

        Box::new(
            ApplicationMenuBar::new(root)
                .menu(
                    ApplicationMenu::new("File")
                        .item(
                            ApplicationMenuItem::new("Open…", move || {
                                menu_open_model.show_open(&menu_panel)
                            })
                            .shortcut(MenuShortcut::command('o', "Ctrl+O")),
                        )
                        .separator()
                        .item(
                            ApplicationMenuItem::new("Close", request_close_key_window)
                                .shortcut(MenuShortcut::command('w', "Ctrl+W")),
                        )
                        .item(
                            ApplicationMenuItem::new("Quit Viewer", request_quit)
                                .shortcut(MenuShortcut::command('q', "Ctrl+Q")),
                        ),
                )
                .menu(
                    ApplicationMenu::new("View")
                        .item(
                            ApplicationMenuItem::new("Zoom In", {
                                let model = self.model.clone();
                                move || model.adjust_zoom(1.25)
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('+', "Ctrl++")),
                        )
                        .item(
                            ApplicationMenuItem::new("Zoom Out", {
                                let model = self.model.clone();
                                move || model.adjust_zoom(0.8)
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('-', "Ctrl+-")),
                        )
                        .separator()
                        .item(
                            ApplicationMenuItem::new("Actual Size", {
                                let model = self.model.clone();
                                move || model.actual_size()
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('1', "Ctrl+1")),
                        )
                        .item(
                            ApplicationMenuItem::new("Fit to Window", {
                                let model = self.model.clone();
                                move || model.fit()
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('0', "Ctrl+0")),
                        )
                        .separator()
                        .item(
                            ApplicationMenuItem::new("Rotate Right", {
                                let model = self.model.clone();
                                move || model.rotate(true)
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('r', "Ctrl+R")),
                        )
                        .item(
                            ApplicationMenuItem::new("Rotate Left", {
                                let model = self.model.clone();
                                move || model.rotate(false)
                            })
                            .enabled(has_document)
                            .shortcut(MenuShortcut::command('l', "Ctrl+L")),
                        )
                        .separator()
                        .item(
                            ApplicationMenuItem::new("Previous Image", {
                                let model = self.model.clone();
                                move || model.navigate(-1)
                            })
                            .enabled(self.model.neighbor(-1).is_some()),
                        )
                        .item(
                            ApplicationMenuItem::new("Next Image", {
                                let model = self.model.clone();
                                move || model.navigate(1)
                            })
                            .enabled(self.model.neighbor(1).is_some()),
                        ),
                ),
        )
    }

    fn handle_platform_message_with_handles(
        &mut self,
        message: &[u8],
        handles: &mut [PlatformFileHandle],
    ) -> bool {
        match document::decode_open_message(message, handles) {
            Ok(Some(opened)) => {
                self.model.open_delegated_or_alert(opened);
                true
            }
            Ok(None) => false,
            Err(error) => {
                self.model
                    .alert
                    .present_error("The document could not be opened.", error.to_string());
                true
            }
        }
    }
}

fn empty_view(open_panel: OpenPanel) -> Box<dyn View> {
    Box::new(
        Surface::pane().content(
            VStack::new()
                .alignment(StackAlignment::Center)
                .distribution(StackDistribution::Center)
                .gap(StackGap::Medium)
                .child(Icon::new(SymbolName::File).size(48.0))
                .child(Text::styled("Open a document", TextRole::TitleMedium))
                .child(
                    Text::body("Drop an image here, or choose one from Files.")
                        .tone(TextTone::Secondary),
                )
                .child(
                    Button::new("Open…")
                        .style(ButtonStyle::Primary)
                        .on_click(move || open_panel.show()),
                ),
        ),
    )
}

fn zoom_for_viewport(image: Size, viewport: Size, fit: bool, zoom: f32) -> f32 {
    if !fit {
        return zoom.clamp(0.000001, MAX_ZOOM);
    }
    let inset = Theme::current().spacing.large * 2.0;
    ((viewport.width - inset).max(1.0) / image.width)
        .min((viewport.height - inset).max(1.0) / image.height)
        .min(MAX_ZOOM)
        .max(0.000001)
}
fn neighbor_path(items: &[PathBuf], current: &Path, offset: isize) -> Option<PathBuf> {
    let index = items
        .iter()
        .position(|path| path.file_name() == current.file_name())?;
    let next = index.checked_add_signed(offset)?;
    items.get(next).cloned()
}
fn sibling_images(path: &Path) -> Result<Vec<PathBuf>, String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut paths: Vec<_> = fs::read_dir(parent)
        .map_err(|error| format!("Folder navigation unavailable: {error}"))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (is_supported_path(&path) && path.is_file()).then_some(path)
        })
        .collect();
    if !paths
        .iter()
        .any(|item| item.file_name() == path.file_name())
    {
        paths.push(path.to_path_buf());
    }
    paths.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(&b.file_name().unwrap_or_default().to_ascii_lowercase())
            .then(a.cmp(b))
    });
    Ok(paths)
}

fn document_argument() -> Option<PathBuf> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|argument| {
            let value = argument.to_string_lossy();
            !value.is_empty()
                && !value.starts_with("--")
                && !value.starts_with("__MNU_")
                && !value.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn is_supported_path(path: &Path) -> bool {
    extension(path).is_some_and(|extension| {
        matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg"
        )
    })
}

fn load_document(path: &Path) -> Result<ViewerDocument, String> {
    let file = fs::File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    load_document_file(file, path)
}

fn load_document_file(file: fs::File, path: &Path) -> Result<ViewerDocument, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(String::from("The selected item is not a file."));
    }
    if metadata.len() > MAX_DOCUMENT_BYTES {
        return Err(String::from("The selected document is larger than 256 MB."));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_DOCUMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    decode_document_bytes(path, &bytes)
}

fn decode_document_bytes(path: &Path, bytes: &[u8]) -> Result<ViewerDocument, String> {
    if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
        return Err("The selected document is larger than 256 MB.".into());
    }
    let extension =
        extension(path).ok_or_else(|| String::from("This file type is not supported."))?;
    if extension == "svg" {
        let svg = SvgData::decode(&bytes).map_err(|error| error.to_string())?;
        return Ok(ViewerDocument {
            path: path.to_path_buf(),
            width: svg.width(),
            height: svg.height(),
            preview: PreviewData::Vector(svg),
            format: "SVG",
            rotated: Rc::new(RefCell::new(None)),
        });
    }
    if !is_supported_path(path) {
        return Err(String::from("This file type is not supported."));
    }
    let image = ImageData::decode(&bytes).map_err(|error| error.to_string())?;
    let (width, height) = image.dimensions();
    let format = match extension.as_str() {
        "png" => "PNG",
        "jpg" | "jpeg" => "JPEG",
        "webp" => "WebP",
        "gif" => "GIF",
        "bmp" => "BMP",
        _ => "Image",
    };
    Ok(ViewerDocument {
        path: path.to_path_buf(),
        preview: PreviewData::Raster(image),
        width: width as f32,
        height: height as f32,
        format,
        rotated: Rc::new(RefCell::new(None)),
    })
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

fn main() -> Result<(), appcore::ViewKitError> {
    appcore::run::<ViewerApp>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_are_case_insensitive() {
        assert!(is_supported_path(Path::new("Photo.PNG")));
        assert!(is_supported_path(Path::new("drawing.svg")));
        assert!(!is_supported_path(Path::new("document.pdf")));
        assert!(!is_supported_path(Path::new("archive.zip")));
    }

    #[test]
    fn arguments_ignore_runtime_metadata() {
        let path = Path::new("/tmp/image.png");
        assert!(is_supported_path(path));
        assert_eq!(extension(path).as_deref(), Some("png"));
    }

    #[test]
    fn bundled_png_can_be_loaded() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("appicon.png");
        let document = load_document(&path).expect("bundled app icon should decode");
        assert!(document.width > 0.0);
        assert!(document.height > 0.0);
    }
}

#[cfg(test)]
mod viewer_workflow_tests {
    use super::*;
    fn folder() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "mochios-viewer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn folder_navigation_opens_real_images_and_stops_at_the_endpoints() {
        let directory = folder();
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#;
        for name in ["a.svg", "b.SVG", "c.svg"] {
            fs::write(directory.join(name), svg).unwrap();
        }
        fs::write(directory.join("notes.txt"), b"ignored").unwrap();
        fs::create_dir(directory.join("directory.png")).unwrap();
        let model = ViewerModel::empty();
        model.open(&directory.join("a.svg")).unwrap();
        assert!(model.neighbor(-1).is_none());
        assert_eq!(model.siblings.get().len(), 3);
        model.navigate(1);
        assert_eq!(model.document.get().unwrap().display_name(), "b.SVG");
        model.rotate(true);
        assert_eq!(model.rotation.get(), 1);
        let document = model.document.get().unwrap();
        assert_eq!(
            document.dimensions(model.rotation.get()),
            Size::new(10.0, 20.0)
        );
        assert!(matches!(
            document.preview(1).unwrap(),
            PreviewData::Vector(_)
        ));
        model.navigate(1);
        assert_eq!(model.rotation.get(), 0);
        assert!(model.fit_to_window.get());
        assert_eq!(model.document.get().unwrap().display_name(), "c.svg");
        assert!(model.neighbor(1).is_none());
        model.open(&directory.join("notes.txt")).unwrap_err();
        assert_eq!(model.document.get().unwrap().display_name(), "c.svg");
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn rotating_never_modifies_the_source_file_and_keeps_vector_resolution() {
        let directory = folder();
        let path = directory.join("image.svg");
        let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="100"><rect width="300" height="100" fill="blue"/></svg>"#;
        fs::write(&path, source).unwrap();
        let document = load_document(&path).unwrap();
        for turns in 1..=4 {
            document.preview(turns).unwrap();
        }
        assert_eq!(fs::read(&path).unwrap(), source);
        assert!(document.rotated.borrow().is_some());
        fs::remove_dir_all(directory).unwrap();
    }
}
