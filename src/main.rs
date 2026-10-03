use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use appcore::prelude::*;

const MIN_ZOOM: f32 = 0.1;
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
}

impl ViewerDocument {
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
}

impl ViewerModel {
    fn new() -> Self {
        let model = Self {
            document: State::new(None),
            zoom: State::new(1.0),
            fit_to_window: State::new(true),
            alert: Alert::new(),
        };
        if let Some(path) = document_argument() {
            model.open_or_alert(path);
        }
        model
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

    fn adjust_zoom(&self, factor: f32) {
        let zoom = if self.fit_to_window.get() {
            1.0
        } else {
            self.zoom.get()
        };
        self.fit_to_window.set(false);
        self.zoom.set((zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM));
    }

    fn actual_size(&self) {
        self.fit_to_window.set(false);
        self.zoom.set(1.0);
    }

    fn fit(&self) {
        self.fit_to_window.set(true);
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

    fn body(&self, context: &ViewContext) -> Self::Body {
        let document = self.model.document.get();
        let has_document = document.is_some();
        let panel_for_open = self.open_panel.clone();
        let menu_panel = self.open_panel.clone();
        let fit_model = self.model.clone();
        let actual_model = self.model.clone();
        let zoom_out_model = self.model.clone();
        let zoom_in_model = self.model.clone();

        let toolbar = Toolbar::new(
            HStack::new()
                .alignment(StackAlignment::Center)
                .gap(StackGap::Small)
                .child(
                    Button::new("Open…")
                        .size(ButtonSize::Small)
                        .on_click(move || panel_for_open.show()),
                )
                .child(Spacer::new())
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
        );

        let content: Box<dyn View> = if let Some(document) = document.as_ref() {
            preview_view(document, &self.model, context)
        } else {
            empty_view(self.open_panel.clone())
        };

        let status = document.as_ref().map_or_else(
            || String::from("Open an image to begin"),
            |document| {
                let zoom = effective_zoom(document, &self.model, context);
                format!(
                    "{}   {} × {}   {:.0}%",
                    document.format,
                    document.width.round() as u32,
                    document.height.round() as u32,
                    zoom * 100.0
                )
            },
        );
        let status_bar = Toolbar::bottom(
            HStack::new()
                .alignment(StackAlignment::Center)
                .child(Text::metadata(status))
                .child(Spacer::new())
                .child(Text::metadata(
                    document
                        .as_ref()
                        .map(ViewerDocument::display_name)
                        .unwrap_or_default(),
                )),
        );

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
                            ApplicationMenuItem::new("Open…", move || menu_panel.show())
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
                            .shortcut(MenuShortcut::command('+', "Ctrl++")),
                        )
                        .item(
                            ApplicationMenuItem::new("Zoom Out", {
                                let model = self.model.clone();
                                move || model.adjust_zoom(0.8)
                            })
                            .shortcut(MenuShortcut::command('-', "Ctrl+-")),
                        )
                        .separator()
                        .item(ApplicationMenuItem::new("Actual Size", {
                            let model = self.model.clone();
                            move || model.actual_size()
                        }))
                        .item(ApplicationMenuItem::new("Fit to Window", {
                            let model = self.model.clone();
                            move || model.fit()
                        })),
                ),
        )
    }

    fn handle_platform_message_with_handles(
        &mut self,
        message: &[u8],
        handles: &[PlatformFileHandle],
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

fn preview_view(
    document: &ViewerDocument,
    model: &ViewerModel,
    context: &ViewContext,
) -> Box<dyn View> {
    let zoom = effective_zoom(document, model, context);
    let width = (document.width * zoom).max(1.0);
    let height = (document.height * zoom).max(1.0);
    let label = document.display_name();
    let preview: Box<dyn View> = match &document.preview {
        PreviewData::Raster(image) => Box::new(
            Image::new(image.clone())
                .content_mode(ImageContentMode::Fit)
                .accessibility_label(label),
        ),
        PreviewData::Vector(svg) => Box::new(
            Svg::new(svg.clone())
                .content_mode(SvgContentMode::Fit)
                .original_colors()
                .accessibility_label(label),
        ),
    };
    Box::new(Surface::pane().content(Scroll::both(preview.layout().frame(width, height))))
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

fn effective_zoom(document: &ViewerDocument, model: &ViewerModel, context: &ViewContext) -> f32 {
    if !model.fit_to_window.get() {
        return model.zoom.get().clamp(MIN_ZOOM, MAX_ZOOM);
    }
    let layout = Theme::current().layout;
    let available_width = (context.size().width - Theme::current().spacing.huge).max(1.0);
    let available_height = (context.size().height
        - layout.top_bar_height
        - layout.bottom_bar_height
        - Theme::current().spacing.huge)
        .max(1.0);
    (available_width / document.width)
        .min(available_height / document.height)
        .clamp(MIN_ZOOM, MAX_ZOOM)
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

fn load_document_file(mut file: fs::File, path: &Path) -> Result<ViewerDocument, String> {
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
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    decode_document_bytes(path, &bytes)
}

fn decode_document_bytes(path: &Path, bytes: &[u8]) -> Result<ViewerDocument, String> {
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
