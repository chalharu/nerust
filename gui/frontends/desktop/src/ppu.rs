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
        })
    }

    pub(crate) fn start(&self, flight: nerust_gui_shell::debug::DrainFlight) {
        if !self.model.is_closed() {
            self.flight.replace(Some(flight));
        }
    }

    pub(crate) fn cancel(&self) {
        self.flight.borrow_mut().take();
    }

    pub(crate) fn poll(&self, language: AppLanguage) {
        if self.model.is_closed() {
            self.cancel();
            return;
        }
        self.language.set(language);
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
    }

    fn refresh(&self, display: DisplayCache) {
        let panels = format_panels(&display.panels);
        let has_panels = !display.panels.is_empty();
        if self.generation.get() == Some(display.images_gen) {
            let _ = self.model.update(move |model| {
                model.panels = panels;
                model.has_panels = has_panels;
            });
            return;
        }
        self.generation.set(Some(display.images_gen));
        let old = self.subscriptions.take();
        drop(old);
        let mut images = Vec::new();
        let mut subscriptions = Vec::new();
        let mut remaining = IMAGE_BUDGET;
        for (index, image) in display.images.into_iter().take(256).enumerate() {
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
            let key = index.to_string();
            let row = Store::new(
                Image {
                    label: label.clone(),
                    style: panechord::Style {
                        horizontal_alignment: Some(panechord::Alignment::Start),
                        placement: panechord::Placement {
                            width: Some(source.width() as f32),
                            height: Some(source.height() as f32),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    source: Some(source),
                    pointer: None,
                },
                self.executor.clone(),
            );
            let output = self.model.clone();
            let language = self.language.clone();
            let hover_key = key.clone();
            subscriptions.push(row.bind(|image| image.pointer, None).subscribe(Rc::new(
                move |point| {
                    let key = hover_key.clone();
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
                },
            )));
            images.push(Keyed::new(key, row));
        }
        self.subscriptions.replace(subscriptions);
        let _ = self.model.update(move |model| {
            model.is_empty = images.is_empty();
            model.images = images;
            model.panels = panels;
            model.has_panels = has_panels;
            model.hovered = None;
            model.hover = PPU_HOVER_HINT.into();
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
