//! Error dialog window: one Shell modal batch (or one action-notice fallback)
//! per window, following the history window pattern. The Shell `ModalQueue`
//! stays the only queue gate; this pane only presents the taken text and
//! closes its own view. ACK (`modal_closed`) and pause release run in the
//! desktop frame loop on this window's close, mirroring Android's
//! `ErrorDialogController::on_dismissed`.
use nerust_settings_core::{
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};
use panechord::backend::View;
use panechord::{Binding, Command, Executor, Store};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Model {
    title: String,
    text: String,
    ok: String,
}

pub(crate) struct Pane {
    model: Store<Model>,
}

impl Pane {
    pub(crate) fn new(
        language: AppLanguage,
        detail: String,
        executor: Rc<dyn Executor>,
    ) -> Rc<Self> {
        Rc::new(Self {
            model: Store::new(
                Model {
                    title: I18n::resolve_os(language).text(UiText::Error).to_owned(),
                    text: detail,
                    ok: I18n::resolve_os(language).text(UiText::Ok).to_owned(),
                },
                executor,
            ),
        })
    }

    pub(crate) fn build(self: &Rc<Self>) -> Result<View, String> {
        let close_view = Rc::new(RefCell::new(None::<panechord::backend::WeakView>));
        let close = close_view.clone();
        let dismiss = self.model.command(
            |_| true,
            move |_, ()| {
                if let Some(view) = close
                    .borrow()
                    .as_ref()
                    .and_then(panechord::backend::WeakView::upgrade)
                {
                    view.close();
                }
            },
            Binding::constant(()),
        );
        let bindings = Presentation {
            title: self.model.bind(|model| model.title.clone(), None),
            text: self.model.bind(|model| model.text.clone(), None),
            ok: self.model.bind(|model| model.ok.clone(), None),
            dismiss: dismiss.clone(),
        };
        let view = panechord::try_view!("ui/error.yaml", bindings, panechord::backend)?;
        close_view.replace(Some(view.downgrade()));
        // Single-button alert: both Escape and Return dismiss, per the
        // GNOME/Apple/Windows single-button convention (`invoke` no-ops
        // while disabled; the button is always enabled here).
        let dismiss_key = dismiss.clone();
        let pane_key = self.clone();
        view.on_surface_event(Rc::new(move |event| {
            if pane_key.model.is_closed() {
                return;
            }
            if let panechord::WindowEvent::KeyboardInput {
                key,
                pressed: true,
                repeat: false,
                synthetic: false,
            } = event
                && (key.as_str() == "Escape"
                    || key.as_str() == "Enter"
                    || key.as_str() == "NumpadEnter")
            {
                let _ = dismiss_key.invoke();
            }
        }));
        let pane = self.clone();
        view.on_close(Rc::new(move || {
            pane.model.close();
        }));
        Ok(view)
    }
}

#[derive(Clone)]
struct Presentation {
    title: Binding<String>,
    text: Binding<String>,
    ok: Binding<String>,
    dismiss: Command,
}
impl Presentation {
    fn command_dismiss(&self, _: Binding<String>) -> Command {
        self.dismiss.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NoDispatch;
    impl Executor for NoDispatch {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("error dialog must not dispatch async work");
        }
    }

    #[test]
    fn error_model_carries_taken_text_in_both_languages() {
        for language in [AppLanguage::English, AppLanguage::Japanese] {
            let pane = Pane::new(language, "boom".into(), Rc::new(NoDispatch));
            let model = pane.model.snapshot();
            assert_eq!(model.text, "boom");
            assert_eq!(model.title, I18n::new(language).text(UiText::Error));
            assert_eq!(model.ok, I18n::new(language).text(UiText::Ok));
            pane.model.close();
        }
    }
}
