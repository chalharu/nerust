//! Error dialog window: one Shell modal batch (or one action-notice fallback)
//! per window, following the history window pattern. The Shell `ModalQueue`
//! stays the only queue gate; this pane only presents the taken text and
//! closes its own view. ACK (`modal_closed`) and pause release run in the
//! desktop frame loop on this window's close, mirroring Android's
//! `ErrorDialogController::on_dismissed`.
use nerust_settings_core::{
    i18n::{UiText, text},
    model::language::AppLanguage,
};
use panechord::{Binding, Command, Executor, Store, View};
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
                    title: text(language, UiText::Error).to_owned(),
                    text: detail,
                    ok: text(language, UiText::Ok).to_owned(),
                },
                executor,
            ),
        })
    }

    pub(crate) fn build(self: &Rc<Self>) -> Result<View, String> {
        let close_view = Rc::new(RefCell::new(None::<panechord::WeakView>));
        let close = close_view.clone();
        let bindings = Presentation {
            title: self.model.bind(|model| model.title.clone(), None),
            text: self.model.bind(|model| model.text.clone(), None),
            ok: self.model.bind(|model| model.ok.clone(), None),
            dismiss: self.model.command(
                |_| true,
                move |_, ()| {
                    if let Some(view) = close
                        .borrow()
                        .as_ref()
                        .and_then(panechord::WeakView::upgrade)
                    {
                        view.close();
                    }
                },
                Binding::constant(()),
            ),
        };
        let view = panechord::try_view!("ui/error.yaml", bindings, panechord)?;
        close_view.replace(Some(view.downgrade()));
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
            assert_eq!(model.title, text(language, UiText::Error));
            assert_eq!(model.ok, text(language, UiText::Ok));
            pane.model.close();
        }
    }
}
