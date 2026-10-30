#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
        task::Poll,
    };
    use wakui2_runtime::{Binding, MenuItem, Session, ShutdownPhase, Store, UiHost, WindowHooks};
    use wakui3_macos::{MacHost, View};

    async fn turn() {
        let mut yielded = false;
        futures::future::poll_fn(move |context| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                context.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
    }

    struct Probe {
        allow: Cell<bool>,
        drained: Cell<bool>,
        confirms: Cell<usize>,
        cancels: Cell<usize>,
        events: RefCell<Vec<&'static str>>,
    }
    fn window(
        host: &MacHost,
        session: &Session,
        id: &str,
        model: &Store<String>,
        probe: &Rc<Probe>,
    ) -> View {
        let view = View::new(Binding::constant(id.into()));
        view.entry(
            &view.root(),
            "edit",
            model.bind(Clone::clone, Some(|state, value| *state = value)),
        );
        let writer = model.clone();
        let name = id.to_owned();
        view.menu(
            "file",
            Binding::constant("File".into()),
            Binding::constant(vec![MenuItem {
                id: "update".into(),
                label: "Update".into(),
                argument: name,
                shortcut: Some("Primary+U".into()),
                role: wakui2_runtime::MenuRole::Normal,
                enabled: true,
            }]),
            Rc::new(move |value| {
                writer.update(move |state| *state = value);
                true
            }),
        );
        let confirm = probe.clone();
        let cancel = probe.clone();
        let drain = probe.clone();
        let closed = probe.clone();
        let weak = view.downgrade();
        host.register_view(
            session,
            id,
            &view,
            WindowHooks {
                confirm: Rc::new(move || {
                    confirm.confirms.set(confirm.confirms.get() + 1);
                    confirm.events.borrow_mut().push("confirm");
                    let allowed = confirm.allow.get();
                    Box::pin(async move { allowed })
                }),
                cancel: Rc::new(move || {
                    cancel.cancels.set(cancel.cancels.get() + 1);
                    cancel.events.borrow_mut().push("cancel");
                }),
                drain: Rc::new(move || {
                    let drain = drain.clone();
                    Box::pin(futures::future::poll_fn(move |context| {
                        if drain.drained.get() {
                            Poll::Ready(())
                        } else {
                            context.waker().wake_by_ref();
                            Poll::Pending
                        }
                    }))
                }),
                close: Rc::new(move || {
                    assert!(closed.drained.get());
                    closed.events.borrow_mut().push("close");
                    if let Some(view) = weak.upgrade() {
                        view.close();
                    }
                }),
            },
        )
        .unwrap();
        view
    }

    pub fn run() -> Result<(), String> {
        wakui3_macos::run_host(|host| {
            let session = Session::new();
            host.attach(session.clone())?;
            let model = Store::new("initial".to_owned(), host.executor());
            let probe = Rc::new(Probe {
                allow: Cell::new(false),
                drained: Cell::new(false),
                confirms: Cell::new(0),
                cancels: Cell::new(0),
                events: RefCell::new(Vec::new()),
            });
            let main = window(&host, &session, "main", &model, &probe);
            let inspector = window(&host, &session, "inspector", &model, &probe);
            let history = window(&host, &session, "history", &model, &probe);
            let executor = host.executor();
            executor.spawn(Box::pin(async move {
                turn().await;
                main.edit("edit", "shared").unwrap();
                assert_eq!(inspector.text("edit").as_deref(), Some("shared"));
                assert_eq!(history.text("edit").as_deref(), Some("shared"));
                inspector.request_close();
                assert!(inspector.downgrade().upgrade().is_none());
                assert!(!model.is_closed());
                assert_eq!(session.window_count(), 2);
                let inspector = window(&host, &session, "inspector", &model, &probe);
                assert_eq!(session.window_count(), 3);
                for (view, expected) in [(&main, "main"), (&inspector, "inspector"), (&history, "history")] {
                    view.present();
                    turn().await;
                    assert!(view.perform_shortcut("Primary+U").unwrap());
                    assert_eq!(model.snapshot(), expected);
                }
                assert!(main.perform_shortcut("Primary+Q").unwrap());
                for _ in 0..4 { turn().await; }
                assert_eq!(session.shutdown().phase(), ShutdownPhase::Running);
                assert_eq!(probe.cancels.get(), 0);
                assert_eq!(session.window_count(), 3);
                probe.allow.set(true);
                probe.events.borrow_mut().clear();
                let before = probe.confirms.get();
                assert!(main.perform_shortcut("Primary+Q").unwrap());
                for _ in 0..4 { turn().await; }
                assert_eq!(session.shutdown().phase(), ShutdownPhase::Draining);
                assert_eq!(probe.confirms.get() - before, 3);
                assert_eq!(probe.cancels.get(), 3);
                assert!(main.downgrade().upgrade().is_some());
                probe.drained.set(true);
                for _ in 0..4 { turn().await; }
                assert_eq!(session.shutdown().phase(), ShutdownPhase::Closed);
                assert_eq!(session.window_count(), 0);
                assert!(main.downgrade().upgrade().is_none());
                assert!(history.downgrade().upgrade().is_none());
                assert!(inspector.downgrade().upgrade().is_none());
                assert_eq!(&*probe.events.borrow(), &["confirm", "confirm", "confirm", "cancel", "cancel", "cancel", "close", "close", "close"]);
                assert!(!model.is_closed());
                model.close();
                println!("MACOS3 PASS multiwindow shared-store reopen active-menu quit-veto confirm-cancel-drain-close");
            }));
            Ok(())
        })
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    native::run().unwrap();
}
