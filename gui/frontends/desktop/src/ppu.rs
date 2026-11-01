use nerust_gui_viewmodel::debugger::{
    DisplayCache, format_panels,
    images::{PPU_HOVER_HINT, scale2x_nearest, tile_hover_text},
};
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use panechord::{Executor, ImageFrame, Keyed, PointerPosition, Store, Subscription, View};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const IMAGE_BUDGET: usize = 64 * 1024 * 1024;
pub(crate) type Fetch = Rc<dyn Fn(Option<u64>) -> nerust_gui_shell::debug::LiveImagesFlight>;

#[derive(Clone)]
struct Image {
    label: String,
    source: Option<ImageFrame>,
    style: panechord::Style,
    pointer: Option<PointerPosition>,
}

fn scaled_image(
    image: nerust_gui_viewmodel::debugger::images::DebugImageRgba,
) -> Result<ImageFrame, &'static str> {
    let width = image.width.checked_mul(2).ok_or("image width overflow")?;
    let height = image.height.checked_mul(2).ok_or("image height overflow")?;
    let length = (width as usize)
        .checked_mul(height as usize)
        .and_then(|length| length.checked_mul(4))
        .ok_or("image size overflow")?;
    if width > 8192 || height > 8192 || length > IMAGE_BUDGET {
        return Err("scaled image exceeds limits");
    }
    let original = ImageFrame::new(image.width, image.height, image.rgba.into())?;
    ImageFrame::new(
        width,
        height,
        scale2x_nearest(original.pixels(), original.width(), original.height()).into(),
    )
}

#[derive(Clone)]
struct Model {
    title: String,
    hover: String,
    hovered: Option<String>,
    images: Vec<Keyed<Image>>,
    panels: String,
    has_panels: bool,
    empty: String,
    is_empty: bool,
}

pub(crate) struct Pane {
    model: Store<Model>,
    executor: Rc<dyn Executor>,
    language: Rc<Cell<AppLanguage>>,
    generation: Cell<Option<u64>>,
    subscriptions: RefCell<Vec<Subscription>>,
    flight: RefCell<Option<nerust_gui_shell::debug::DrainFlight>>,
    live: RefCell<Option<nerust_gui_shell::debug::LiveImagesFlight>>,
    fetch: RefCell<Option<Fetch>>,
    next_fetch: Cell<std::time::Instant>,
    media_revision: Cell<Option<u64>>,
}

impl Pane {
    pub(crate) fn new(language: AppLanguage, executor: Rc<dyn Executor>) -> Rc<Self> {
        Rc::new(Self {
            model: Store::new(
                Model {
                    title: text(language, UiText::PpuViewer).into(),
                    hover: PPU_HOVER_HINT.into(),
                    hovered: None,
                    images: vec![],
                    panels: String::new(),
                    has_panels: false,
                    empty: text(language, UiText::NoImages).into(),
                    is_empty: true,
                },
                executor.clone(),
            ),
            executor,
            language: Rc::new(Cell::new(language)),
            generation: Cell::new(None),
            subscriptions: RefCell::new(vec![]),
            flight: RefCell::new(None),
            live: RefCell::new(None),
            fetch: RefCell::new(None),
            next_fetch: Cell::new(std::time::Instant::now()),
            media_revision: Cell::new(None),
        })
    }

    pub(crate) fn start(&self, flight: nerust_gui_shell::debug::DrainFlight) {
        if !self.model.is_closed() {
            self.flight.replace(Some(flight));
        }
    }

    pub(crate) fn connect(&self, media_revision: u64, fetch: Fetch) {
        self.media_revision.set(Some(media_revision));
        self.fetch.replace(Some(fetch));
    }

    pub(crate) fn cancel(&self) {
        self.flight.borrow_mut().take();
        self.live.borrow_mut().take();
        self.fetch.borrow_mut().take();
    }

    pub(crate) fn is_idle(&self) -> bool {
        self.flight.borrow().is_none() && self.live.borrow().is_none()
    }

    pub(crate) fn poll(&self, language: AppLanguage, loaded: bool, media_revision: u64) {
        if self.model.is_closed() {
            self.cancel();
            return;
        }
        if self.language.replace(language) != language {
            let _ = self.model.update(move |model| {
                model.title = text(language, UiText::PpuViewer).into();
                model.empty = text(language, UiText::NoImages).into();
            });
        }
        if self.media_revision.replace(Some(media_revision)) != Some(media_revision) {
            self.flight.borrow_mut().take();
            self.live.borrow_mut().take();
            self.generation.set(None);
            self.refresh(DisplayCache::default());
            self.generation.set(None);
        }
        if !loaded {
            if !self.model.snapshot().is_empty {
                self.flight.borrow_mut().take();
                self.live.borrow_mut().take();
                self.generation.set(None);
                self.refresh(DisplayCache::default());
            }
            return;
        }
        let output = {
            let mut flight = self.flight.borrow_mut();
            match flight.as_mut().map(|flight| flight.poll()) {
                Some(nerust_gui_shell::debug::DrainPoll::Done(output)) => {
                    flight.take();
                    Some(*output)
                }
                _ => None,
            }
        };
        if let Some(output) = output {
            self.refresh(output.display);
        }
        if self.flight.borrow().is_some() {
            return;
        }
        let images = {
            let mut live = self.live.borrow_mut();
            match live.as_mut().map(|flight| flight.poll()) {
                Some(nerust_gui_shell::debug::LiveImagesPoll::Done(images)) => {
                    live.take();
                    self.next_fetch
                        .set(std::time::Instant::now() + std::time::Duration::from_millis(100));
                    images
                }
                _ => None,
            }
        };
        if let Some(images) = images
            && self.generation.get() != Some(images.generation)
        {
            self.refresh_images(
                images
                    .images
                    .into_iter()
                    .map(
                        |image| nerust_gui_viewmodel::debugger::images::DebugImageRgba {
                            label: image.label,
                            width: image.width,
                            height: image.height,
                            rgba: image.rgba,
                        },
                    )
                    .collect(),
                images.generation,
            );
        }
        if self.live.borrow().is_none()
            && std::time::Instant::now() >= self.next_fetch.get()
            && let Some(fetch) = self.fetch.borrow().clone()
        {
            self.live.replace(Some(fetch(self.generation.get())));
        }
    }

    fn refresh(&self, display: DisplayCache) {
        let panels = format_panels(&display.panels);
        let has_panels = !display.panels.is_empty();
        let _ = self.model.update(move |model| {
            model.panels = panels;
            model.has_panels = has_panels;
        });
        self.refresh_images(display.images, display.images_gen);
    }

    fn refresh_images(
        &self,
        source_images: Vec<nerust_gui_viewmodel::debugger::images::DebugImageRgba>,
        generation: u64,
    ) {
        if self.generation.get() == Some(generation) {
            return;
        }
        self.generation.set(Some(generation));
        let old = self.subscriptions.take();
        drop(old);
        let mut images = Vec::new();
        let mut subscriptions = Vec::new();
        let mut previous: std::collections::BTreeMap<_, _> = self
            .model
            .snapshot()
            .images
            .into_iter()
            .map(|image| (image.key.clone(), image.model))
            .collect();
        let mut labels = std::collections::BTreeMap::<String, usize>::new();
        let mut remaining = IMAGE_BUDGET;
        for image in source_images.into_iter().take(256) {
            let Some(length) = (image.width as usize)
                .checked_mul(image.height as usize)
                .and_then(|length| length.checked_mul(16))
            else {
                continue;
            };
            let Some(budget) = remaining.checked_sub(length) else {
                continue;
            };
            let label = image.label.clone();
            let (width, height) = (image.width, image.height);
            let Ok(source) = scaled_image(image) else {
                continue;
            };
            remaining = budget;
            let identity: String = label.bytes().map(|byte| format!("{byte:02x}")).collect();
            let occurrence = labels.entry(identity.clone()).or_default();
            let key = format!("image_{identity}_{occurrence}");
            *occurrence += 1;
            let style = panechord::Style {
                horizontal_alignment: Some(panechord::Alignment::Start),
                placement: panechord::Placement {
                    width: Some(source.width() as f32),
                    height: Some(source.height() as f32),
                    ..Default::default()
                },
                ..Default::default()
            };
            let row = if let Some(row) = previous.remove(&key) {
                let label = label.clone();
                let _ = row.update(move |image| {
                    image.label = label;
                    image.style = style;
                    image.source = Some(source);
                });
                row
            } else {
                Store::new(
                    Image {
                        label: label.clone(),
                        style,
                        source: Some(source),
                        pointer: None,
                    },
                    self.executor.clone(),
                )
            };
            let output = self.model.clone();
            let language = self.language.clone();
            let hover_key = key.clone();
            subscriptions.push(
                row.bind(
                    |image| {
                        (
                            image.pointer,
                            image.label.clone(),
                            image
                                .source
                                .as_ref()
                                .map(|source| (source.width() / 2, source.height() / 2)),
                        )
                    },
                    None,
                )
                .subscribe(Rc::new(move |(point, label, dimensions)| {
                    let key = hover_key.clone();
                    let (width, height) = dimensions.unwrap_or((width, height));
                    let caption = point.map(|point| {
                        tile_hover_text(
                            language.get(),
                            &label,
                            width,
                            height,
                            f64::from(point.x),
                            f64::from(point.y),
                        )
                    });
                    let _ = output.update(move |model| {
                        if let Some(caption) = caption {
                            model.hovered = Some(key);
                            model.hover = caption;
                        } else if model.hovered.as_ref() == Some(&key) {
                            model.hovered = None;
                            model.hover = PPU_HOVER_HINT.into();
                        }
                    });
                })),
            );
            images.push(Keyed::new(key, row));
        }
        self.subscriptions.replace(subscriptions);
        let _ = self.model.update(move |model| {
            model.is_empty = images.is_empty();
            if model
                .hovered
                .as_ref()
                .is_some_and(|key| !images.iter().any(|image| &image.key == key))
            {
                model.hovered = None;
                model.hover = PPU_HOVER_HINT.into();
            }
            model.images = images;
        });
    }

    pub(crate) fn build(self: &Rc<Self>) -> Result<View, String> {
        let view = panechord::try_view!("ui/ppu.yaml", self.model, panechord)?;
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.cancel();
            let subscriptions = pane.subscriptions.take();
            drop(subscriptions);
            pane.model.close();
        }));
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_viewmodel::debugger::images::DebugImageRgba;
    struct NoDispatch;
    impl Executor for NoDispatch {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("image projection must not spawn work");
        }
    }

    #[test]
    fn live_image_generation_reuses_identity_and_preserves_hover() {
        let pane = Pane::new(AppLanguage::English, Rc::new(NoDispatch));
        let image = |value| DebugImageRgba {
            label: "pattern".into(),
            width: 1,
            height: 1,
            rgba: vec![value, 0, 0, 255],
        };
        pane.refresh_images(vec![image(1)], 1);
        let first = pane.model.snapshot().images[0].clone();
        let _ = first
            .model
            .update(|image| image.pointer = Some(PointerPosition { x: 0.0, y: 0.0 }));
        let hover = pane.model.snapshot().hover;
        pane.refresh_images(vec![image(2)], 2);
        let second = pane.model.snapshot().images[0].clone();
        assert_eq!(first.key, second.key);
        assert!(first.model.ptr_eq(&second.model));
        assert_eq!(pane.model.snapshot().hover, hover);
        assert_eq!(second.model.snapshot().source.unwrap().pixels()[0], 2);
        pane.refresh_images(vec![image(3)], 2);
        assert_eq!(second.model.snapshot().source.unwrap().pixels()[0], 2);
        pane.refresh_images(Vec::new(), 3);
        assert!(pane.model.snapshot().is_empty);
        assert!(pane.model.snapshot().hovered.is_none());
    }

    #[test]
    fn scaling_validates_shape_and_preserves_nearest_pixels() {
        let frame = scaled_image(DebugImageRgba {
            label: "test".into(),
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        })
        .unwrap();
        assert_eq!((frame.width(), frame.height()), (2, 2));
        assert_eq!(&**frame.pixels(), &[1, 2, 3, 255].repeat(4));
        assert!(
            scaled_image(DebugImageRgba {
                label: "bad".into(),
                width: 1,
                height: 1,
                rgba: vec![]
            })
            .is_err()
        );
        assert!(
            scaled_image(DebugImageRgba {
                label: "huge".into(),
                width: 8192,
                height: 8192,
                rgba: vec![]
            })
            .is_err()
        );
    }
}
