//! PPU viewer definition: chrome, pictures, and generic control projection.
//!
//! The viewer renders as a generic [`wakui::scene::control::ControlScreen`] with
//! image leaves; native frontends never
//! name this module. Hover positions arrive as generic
//! [`wakui::scene::control::ControlInput::Hover`] events and are formatted here.

use nerust_gui_viewmodel::debugger::{
    DisplayCache, format_panels, scale2x_nearest, tile_hover_text,
};
use nerust_settings_core::model::language::AppLanguage;
use wakui::scene::control::{
    Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlEvent, ControlId,
    ControlInput, ControlMetadata, ControlScreen, IdRegistry, Node, Page,
};
use wakui::scene::leaf::{ActionLeaf, ImageLeaf, NoteLeaf};

/// Pre-localized PPU viewer chrome. Frontends render verbatim.
#[derive(Debug, Clone, Default)]
pub struct PpuChrome {
    pub title: String,
    pub hover_empty: String,
    pub no_images: String,
    pub panels_title: String,
}

/// One 2x-scaled PPU picture with its display size.
#[derive(Debug, Clone)]
pub struct PpuPicture {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Pre-localized PPU viewer chrome in the view language.
pub fn ppu_chrome(language: AppLanguage) -> PpuChrome {
    use nerust_settings_core::i18n::{UiText, text as ui_text};
    PpuChrome {
        title: ui_text(language, UiText::PpuViewer).to_string(),
        hover_empty: ui_text(language, UiText::HoverTile).to_string(),
        no_images: ui_text(language, UiText::NoImages).to_string(),
        panels_title: ui_text(language, UiText::Panels).to_string(),
    }
}

/// 2x-scaled pictures derived without exposing the scaler.
pub fn scaled_pictures(display: &DisplayCache) -> Vec<PpuPicture> {
    display
        .images
        .iter()
        .map(|image| {
            let rgba = scale2x_nearest(&image.rgba, image.width, image.height);
            PpuPicture {
                label: image.label.clone(),
                width: image.width * 2,
                height: image.height * 2,
                rgba,
            }
        })
        .collect()
}

/// Formatted panel text without exposing the panel formatter.
pub fn panels_text(display: &DisplayCache) -> String {
    format_panels(&display.panels)
}

fn layout_id(ids: &mut IdRegistry<String>, key: &str) -> wakui::scene::control::LayoutId {
    ids.id(key.to_string())
}

fn control_id(ids: &mut IdRegistry<String>, key: &str) -> ControlId {
    ids.id(key.to_string())
}

fn note_control(
    ids: &mut IdRegistry<String>,
    key: &str,
    label: &str,
    text: String,
    display: ControlDisplay,
) -> Control {
    Control {
        id: control_id(ids, key),
        label: label.to_string(),
        metadata: ControlMetadata::new(label),
        display,
        leaf: Box::new(NoteLeaf { text }),
    }
}

fn image_control(ids: &mut IdRegistry<String>, key: &str, picture: &PpuPicture) -> Control {
    Control {
        id: control_id(ids, &format!("image:{key}")),
        label: picture.label.clone(),
        metadata: ControlMetadata::new(picture.label.clone()),
        display: ControlDisplay::Standard,
        leaf: Box::new(ImageLeaf {
            width: picture.width,
            height: picture.height,
            rgba: picture.rgba.clone(),
        }),
    }
}

fn column_layout(ids: &mut IdRegistry<String>, key: &str, children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new(key);
    metadata.hints.spacing = Some(12.0);
    metadata.hints.indent = Some(0.0);
    Container {
        id: layout_id(ids, key),
        label: String::new(),
        metadata,
        header: None,
        policy: Box::new(ColumnPolicy),
        children,
    }
}

/// Owned PPU viewer session: pictures plus hover line, projected as controls.
#[derive(Debug, Default)]
pub struct PpuSession {
    ids: IdRegistry<String>,
    chrome: PpuChrome,
    hover: String,
    pictures: Vec<PpuPicture>,
    panels: String,
    has_panels: bool,
    generation: u64,
}

impl PpuSession {
    pub fn new(
        chrome: PpuChrome,
        pictures: Vec<PpuPicture>,
        panels: Option<String>,
        generation: u64,
    ) -> Self {
        let has_panels = panels.is_some();
        Self {
            ids: IdRegistry::default(),
            hover: chrome.hover_empty.clone(),
            chrome,
            pictures,
            panels: panels.unwrap_or_default(),
            has_panels,
            generation,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Refresh from a new display snapshot. Returns true when the projected
    /// screen changed (pictures, panels, or chrome).
    pub fn refresh(
        &mut self,
        chrome: PpuChrome,
        pictures: Vec<PpuPicture>,
        panels: Option<String>,
        generation: u64,
    ) -> bool {
        let changed = self.generation != generation
            || self.pictures.len() != pictures.len()
            || self.panels != panels.as_deref().unwrap_or_default()
            || self.has_panels != panels.is_some()
            || self.chrome.title != chrome.title
            || self.chrome.hover_empty != chrome.hover_empty
            || self.chrome.no_images != chrome.no_images
            || self.chrome.panels_title != chrome.panels_title;
        // Hover resets to the (possibly re-localized) empty line on refresh,
        // mirroring the previous native viewers which kept hover separate
        // from the pushed pictures.
        self.chrome = chrome;
        self.hover.clone_from(&self.chrome.hover_empty);
        self.pictures = pictures;
        self.panels = panels.unwrap_or_default();
        self.has_panels = !self.panels.is_empty() || self.has_panels && self.panels.is_empty();
        self.generation = generation;
        changed
    }

    /// Format a hover position for an image control. Returns the new line.
    pub fn hover_line(
        &mut self,
        language: AppLanguage,
        label: &str,
        width: u32,
        height: u32,
        x: u32,
        y: u32,
    ) -> String {
        // Geometry arrives halved back: controls carry display (2x) size,
        // tile math expects source size.
        let line = tile_hover_text(language, label, width, height, f64::from(x), f64::from(y));
        self.hover = line.clone();
        line
    }

    fn hover_text(&self) -> String {
        if self.hover.is_empty() {
            self.chrome.hover_empty.clone()
        } else {
            self.hover.clone()
        }
    }

    /// Project the current viewer state as a generic control screen.
    pub fn screen(&mut self) -> ControlScreen {
        let hover = self.hover_text();
        let no_images = self.chrome.no_images.clone();
        let panels_title = self.chrome.panels_title.clone();
        let panels = self.panels.clone();
        let title = self.chrome.title.clone();
        let mut children = Vec::new();
        children.push(Node::Control(note_control(
            &mut self.ids,
            "hover",
            "",
            hover,
            ControlDisplay::Standard,
        )));
        if self.pictures.is_empty() {
            children.push(Node::Control(note_control(
                &mut self.ids,
                "empty",
                "",
                no_images.clone(),
                ControlDisplay::Standard,
            )));
        }
        for (index, picture) in self.pictures.iter().enumerate() {
            let key = format!("{index}:{}", picture.label);
            children.push(Node::Control(image_control(&mut self.ids, &key, picture)));
        }
        if self.has_panels {
            children.push(Node::Control(note_control(
                &mut self.ids,
                "panels-title",
                "",
                panels_title,
                ControlDisplay::Heading,
            )));
            children.push(Node::Control(note_control(
                &mut self.ids,
                "panels",
                "",
                panels,
                ControlDisplay::Standard,
            )));
        }
        let body = column_layout(&mut self.ids, "body", children);
        let footer = column_layout(&mut self.ids, "footer", Vec::new());
        let page_id = layout_id(&mut self.ids, "page");
        ControlScreen {
            title: title.clone(),
            pages: vec![Page {
                id: page_id,
                label: title.clone(),
                selected: true,
                select: Control {
                    id: control_id(&mut self.ids, "page-select"),
                    label: title.clone(),
                    metadata: ControlMetadata::new(title.clone()),
                    display: ControlDisplay::Standard,
                    leaf: Box::new(ActionLeaf),
                },
                body,
            }],
            footer,
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        }
    }

    /// Apply a generic control event. Hover over an image refreshes the
    /// hover line; returns true when the screen changed.
    pub fn dispatch(
        &mut self,
        screen: &ControlScreen,
        event: &ControlEvent,
        language: AppLanguage,
    ) -> bool {
        let ControlInput::Hover { x, y } = event.input else {
            return false;
        };
        let Some(id) = event.id else { return false };
        let Some(control) = screen.control(id) else {
            return false;
        };
        let Some(image) = control.downcast::<ImageLeaf>() else {
            return false;
        };
        let (width, height) = (image.width, image.height);
        // Source geometry is half the displayed size (2x scale).
        let (src_w, src_h) = (width / 2, height / 2);
        let line = tile_hover_text(
            language,
            &control.label,
            src_w.max(1),
            src_h.max(1),
            f64::from(x) / 2.0,
            f64::from(y) / 2.0,
        );
        if line == self.hover {
            return false;
        }
        self.hover = line;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome() -> PpuChrome {
        PpuChrome {
            title: "PPU Viewer".into(),
            hover_empty: "hover".into(),
            no_images: "empty".into(),
            panels_title: "Panels".into(),
        }
    }

    fn picture(label: &str) -> PpuPicture {
        PpuPicture {
            label: label.into(),
            width: 4,
            height: 4,
            rgba: vec![0xff; 4 * 4 * 4],
        }
    }

    #[test]
    fn empty_screen_shows_placeholder_and_hover() {
        let mut session = PpuSession::new(chrome(), vec![], None, 0);
        let screen = session.screen();
        assert_eq!(screen.title, "PPU Viewer");
        let texts: Vec<_> = collect_notes(&screen);
        assert!(texts.contains(&"hover".to_string()));
        assert!(texts.contains(&"empty".to_string()));
    }

    #[test]
    fn pictures_project_as_stable_image_controls() {
        let mut session = PpuSession::new(
            chrome(),
            vec![picture("Tiles"), picture("Sprites")],
            None,
            1,
        );
        let first = session.screen();
        let ids: Vec<_> = collect_images(&first);
        assert_eq!(ids.len(), 2);
        // Same labels keep stable IDs across refreshes.
        session.refresh(
            chrome(),
            vec![picture("Tiles"), picture("Sprites")],
            None,
            2,
        );
        let second = session.screen();
        assert_eq!(collect_images(&second), ids);
    }

    #[test]
    fn hover_dispatch_updates_only_on_change() {
        let mut session = PpuSession::new(chrome(), vec![picture("Tiles")], None, 1);
        let screen = session.screen();
        let target = collect_images(&screen)[0];
        let event = ControlEvent::new(target, ControlInput::Hover { x: 1, y: 1 });
        assert!(session.dispatch(&screen, &event, AppLanguage::English));
        let line = session.hover.clone();
        // Same hover repeats: no change.
        assert!(!session.dispatch(&screen, &event, AppLanguage::English));
        assert_eq!(session.hover, line);
        // Non-image targets never update.
        let screen = session.screen();
        let note = screen.pages[0]
            .body
            .children
            .iter()
            .find_map(|node| match node {
                Node::Control(c) if c.downcast::<NoteLeaf>().is_some() => Some(c.id),
                _ => None,
            })
            .unwrap();
        assert!(!session.dispatch(
            &screen,
            &ControlEvent::new(note, ControlInput::Hover { x: 1, y: 1 }),
            AppLanguage::English
        ));
    }

    #[test]
    fn panels_section_appears_only_when_present() {
        let mut session = PpuSession::new(chrome(), vec![], None, 0);
        assert!(
            !collect_notes(&session.screen())
                .iter()
                .any(|t| t == "Panels")
        );
        session.refresh(chrome(), vec![], Some("OAM: 1".to_string()), 1);
        assert!(
            collect_notes(&session.screen())
                .iter()
                .any(|t| t == "Panels")
        );
    }

    fn collect_notes(screen: &ControlScreen) -> Vec<String> {
        let mut out = Vec::new();
        for page in &screen.pages {
            collect_layout_notes(&page.body, &mut out);
        }
        collect_layout_notes(&screen.footer, &mut out);
        out
    }

    fn collect_layout_notes(layout: &Container, out: &mut Vec<String>) {
        if let Some(header) = &layout.header
            && let Some(note) = header.downcast::<NoteLeaf>()
        {
            out.push(note.text.clone());
        }
        for node in &layout.children {
            match node {
                Node::Control(c) => {
                    if let Some(note) = c.downcast::<NoteLeaf>() {
                        out.push(note.text.clone());
                    }
                }
                Node::Container(l) => collect_layout_notes(l, out),
                Node::Tabs(tabs) => {
                    for tab in &tabs.tabs {
                        collect_layout_notes(&tab.body, out);
                    }
                }
            }
        }
    }

    fn collect_images(screen: &ControlScreen) -> Vec<ControlId> {
        let mut out = Vec::new();
        for page in &screen.pages {
            collect_layout_images(&page.body, &mut out);
        }
        out
    }

    fn collect_layout_images(layout: &Container, out: &mut Vec<ControlId>) {
        for node in &layout.children {
            match node {
                Node::Control(c) => {
                    if c.downcast::<ImageLeaf>().is_some() {
                        out.push(c.id);
                    }
                }
                Node::Container(l) => collect_layout_images(l, out),
                Node::Tabs(_) => {}
            }
        }
    }
}
