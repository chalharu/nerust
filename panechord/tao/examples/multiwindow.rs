use panechord_runtime::{Binding, Session, ShutdownOutcome, Store, UiHost, WindowHooks};
use panechord_tao::{View, run_host};
use std::{cell::Cell, rc::Rc, time::Duration};

async fn tick() {
    let (send, receive) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let _ = send.send(());
    });
    receive.await.unwrap();
}

async fn drawn(views: &[View]) {
    for _ in 0..100 {
        if views.iter().all(|view| view.frame_count() > 0) {
            return;
        }
        tick().await;
    }
    panic!("multiwindow views did not draw");
}

fn hooks(view: &View, allow: Rc<Cell<bool>>) -> WindowHooks {
    let weak = view.downgrade();
    WindowHooks {
        confirm: Rc::new(move || {
            let allowed = allow.get();
            Box::pin(async move { allowed })
        }),
        cancel: Rc::new(|| {}),
        drain: Rc::new(|| Box::pin(async {})),
        close: Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.close();
            }
        }),
    }
}

fn view(title: &str, model: &Store<u64>) -> View {
    let view = View::new(Binding::constant(title.into()));
    view.label(
        &view.root(),
        "counter",
        model.bind(|value| value.to_string(), None),
    );
    view
}

fn main() -> Result<(), String> {
    run_host(|host| {
        let closed = Session::new();
        closed.shutdown().close_now();
        assert!(host.attach(closed).is_err());
        let session = Session::new();
        host.attach(session.clone())?;
        host.attach(session.clone())?;
        assert!(host.attach(Session::new()).is_err());
        let model = Store::new(1_u64, host.executor());
        let allow = Rc::new(Cell::new(false));
        let primary = view("Tao primary", &model);
        let auxiliary = view("Tao auxiliary", &model);
        let native_ready = Rc::new(Cell::new(0));
        for view in [&primary, &auxiliary] {
            let ready = native_ready.clone();
            view.on_native_window(move |window| {
                assert!(window.inner_size().width > 0);
                ready.set(ready.get() + 1);
                Ok(())
            })?;
            assert!(view.on_native_window(|_| Ok(())).is_err());
        }
        host.register_view(&session, "main", &primary, hooks(&primary, allow.clone()))?;
        host.register_view(
            &session,
            "aux",
            &auxiliary,
            hooks(&auxiliary, allow.clone()),
        )?;
        host.present(&primary);
        let executor = host.executor();
        executor.spawn(Box::pin(async move {
            drawn(&[primary.clone(), auxiliary.clone()]).await;
            assert_eq!(native_ready.get(), 2);
            assert!(primary.on_native_window(|_| Ok(())).is_err());
            assert_eq!(session.shutdown().window_count(), 2);
            primary.close();
            tick().await;
            assert_eq!(session.shutdown().window_count(), 1);
            assert!(!model.is_closed());
            assert!(model.update(|value| *value = 2));
            assert_eq!(auxiliary.text("counter").as_deref(), Some("2"));
            let reopened = view("Tao reopened", &model);
            host.register_view(&session, "main", &reopened, hooks(&reopened, allow.clone())).unwrap();
            drawn(std::slice::from_ref(&reopened)).await;
            assert_eq!(reopened.text("counter").as_deref(), Some("2"));
            assert!(host.register_view(&Session::new(), "foreign", &reopened, hooks(&reopened, allow.clone())).is_err());
            assert!(host.register_view(&session, "closed", &primary, hooks(&primary, allow.clone())).is_err());
            auxiliary.request_quit();
            tick().await;
            assert_eq!(session.shutdown().request_shutdown(Duration::from_millis(100)).await, ShutdownOutcome::Cancelled);
            assert_eq!(session.shutdown().window_count(), 2);
            auxiliary.report_error("rejected auxiliary menu update".into());
            auxiliary.close();
            tick().await;
            assert_eq!(session.shutdown().window_count(), 1);
            assert!(model.update(|value| *value = 3));
            assert_eq!(reopened.text("counter").as_deref(), Some("3"));
            let auxiliary = view("Tao auxiliary reopened", &model);
            host.register_view(&session, "aux", &auxiliary, hooks(&auxiliary, allow.clone())).unwrap();
            drawn(std::slice::from_ref(&auxiliary)).await;
            assert_eq!(session.shutdown().window_count(), 2);
            allow.set(true);
            let shutdown = session.shutdown();
            reopened.on_close(Rc::new(move || {
                assert_eq!(shutdown.window_count(), 0);
                model.close();
                println!("TAO3 PASS multiwindow drawn shared-store primary-close reopen foreign-and-closed-rejected veto-drain-close recoverable-auxiliary-error-isolated");
            }));
            reopened.request_quit();
        }));
        Ok(())
    })
}
