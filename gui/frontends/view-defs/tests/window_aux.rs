use std::{cell::RefCell, sync::Arc};

use wakui::control::{
    Control, ControlEvent, ControlId, ControlInput, ControlScreen, ControlVisitor, Page,
};
use wakui::leaf::NumberLeaf;

use nerust_frontend_viewdefs::{
    focus::AuxKind, settings::SettingsController, widget_session::WidgetSession,
};
use wakui::{
    aux::{AuxId, AuxWindows, OpenRequest},
    window::*,
};

fn session() -> WidgetSession {
    WidgetSession::from_settings(SettingsController::new(
        &nerust_gui_runtime::settings::SettingsSnapshot {
            shared: Default::default(),
            local: Default::default(),
            app_state: Default::default(),
        },
        Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![])),
        Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
    ))
}

fn aux_id(index: u64) -> AuxId {
    AuxId::from_raw(index)
}

#[test]
fn defaults_visit_each_group_with_balanced_outer_structure() {
    #[derive(Default)]
    struct Visitor(Vec<&'static str>);
    impl WindowVisitor for Visitor {
        fn begin_window(&mut self, _: &Window) {
            self.0.push("window");
        }
        fn end_window(&mut self, _: &Window) {
            self.0.push("/window");
        }
        fn begin_widget(&mut self, _: &Widget) {
            self.0.push("widget");
        }
        fn end_widget(&mut self, _: &Widget) {
            self.0.push("/widget");
        }
        fn visit_main(&mut self, _: &MainContents) {
            self.0.push("main");
        }
        fn visit_pending_controls(&mut self) {
            self.0.push("pending");
        }
    }
    // All aux windows start pending; view definitions supply the screens.
    let mut index = 0u64;
    let mut aux = |kind: AuxKind| {
        index += 1;
        (Window::auxiliary(aux_id(index), kind.title()), "pending")
    };
    for (window, group) in [
        (Window::main("test"), "main"),
        aux(AuxKind::Debug),
        aux(AuxKind::Ppu),
        aux(AuxKind::History),
        aux(AuxKind::Settings),
    ] {
        assert!(!window.title.is_empty());
        assert!(window.menu.is_none());
        let mut visitor = Visitor::default();
        let dynamic: &mut dyn WindowVisitor = &mut visitor;
        window.visit(dynamic);
        assert_eq!(visitor.0, ["window", "widget", group, "/widget", "/window"]);
    }
}

#[test]
fn settings_window_owns_latest_tree_and_nested_dyn_visitors_allow_reentry() {
    struct Visitor<'a> {
        session: &'a RefCell<WidgetSession>,
        volume: ControlId,
        pages: usize,
        controls: usize,
    }
    impl WindowVisitor for Visitor<'_> {
        fn visit_controls(&mut self, screen: &ControlScreen) {
            self.session
                .borrow_mut()
                .dispatch(ControlEvent::new(self.volume, ControlInput::Number(19)));
            let nested = self.session.borrow().window(aux_id(7));
            assert!(matches!(
                nested.contents.contents,
                WidgetContents::Controls(_)
            ));
            let dynamic: &mut dyn ControlVisitor = self;
            screen.visit(dynamic);
        }
    }
    impl ControlVisitor for Visitor<'_> {
        fn begin_page(&mut self, _: &Page) {
            self.pages += 1;
        }
        fn control(&mut self, _: &Control) {
            self.controls += 1;
        }
    }
    #[derive(Default)]
    struct Volume(Option<ControlId>);
    impl ControlVisitor for Volume {
        fn control(&mut self, control: &Control) {
            if control
                .downcast::<NumberLeaf>()
                .is_some_and(|leaf| leaf.max == 100)
            {
                self.0 = Some(control.id);
            }
        }
    }
    let session = RefCell::new(session());
    let mut volume = Volume::default();
    session.borrow().screen().visit(&mut volume);
    let volume = volume.0.unwrap();
    session
        .borrow_mut()
        .dispatch(ControlEvent::new(volume, ControlInput::Number(37)));
    let window = session.borrow().window(aux_id(7));
    assert_eq!(window.id, WindowId::Auxiliary(aux_id(7)));
    let WidgetContents::Controls(screen) = &window.contents.contents else {
        panic!("generic tree must be owned by the widget");
    };
    assert_eq!(window.title, screen.title);
    let mut visitor = Visitor {
        session: &session,
        volume,
        pages: 0,
        controls: 0,
    };
    window.visit(&mut visitor as &mut dyn WindowVisitor);
    assert_eq!(visitor.pages, 6);
    assert!(visitor.controls > 20);
    assert_eq!(
        screen
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        37
    );
    let latest = session.borrow().window(aux_id(7));
    drop(session);
    let WidgetContents::Controls(screen) = latest.contents.contents else {
        panic!()
    };
    assert_eq!(
        screen
            .control(volume)
            .unwrap()
            .downcast::<NumberLeaf>()
            .unwrap()
            .value,
        19
    );
}

#[test]
fn reservation_owns_group_before_native_creation_and_reentrant_open() {
    let windows = RefCell::new(AuxWindows::<AuxKind, u64, ()>::new());
    for kind in [AuxKind::Debug, AuxKind::Ppu] {
        let OpenRequest::Create(id) = windows.borrow_mut().request_open(kind, kind.title()) else {
            panic!()
        };
        let snapshot = windows.borrow().window(id).unwrap().clone();
        assert_eq!(snapshot.id, WindowId::Auxiliary(id));
        assert!(matches!(
            snapshot.contents.contents,
            WidgetContents::PendingControls
        ));
        struct Reenter<'a> {
            windows: &'a RefCell<AuxWindows<AuxKind, u64, ()>>,
            id: wakui::aux::AuxId,
        }
        impl WindowVisitor for Reenter<'_> {
            fn begin_window(&mut self, window: &Window) {
                let WindowId::Auxiliary(id) = window.id else {
                    panic!()
                };
                let kind = self.windows.borrow().kind_of_id(id).unwrap();
                assert_eq!(
                    self.windows.borrow_mut().request_open(kind, kind.title()),
                    OpenRequest::Pending(self.id)
                );
            }
        }
        snapshot.visit(&mut Reenter {
            windows: &windows,
            id,
        });
        windows.borrow_mut().window_mut(id).unwrap().title = "owned title".into();
        assert_eq!(windows.borrow_mut().complete_open(id, None, ()), Ok(()));
        assert_eq!(windows.borrow().window(id).unwrap().title, "owned title");
        assert_eq!(
            windows.borrow_mut().request_open(kind, kind.title()),
            OpenRequest::Present(id)
        );
    }
}

#[test]
fn blueprint_lifetime_matches_reservation_retry_removal_and_clear() {
    let mut windows = AuxWindows::<AuxKind, u64, ()>::new();
    let OpenRequest::Create(old) =
        windows.request_open(AuxKind::Settings, AuxKind::Settings.title())
    else {
        panic!()
    };
    let id = old;
    *windows.window_mut(old).unwrap() = session().window(id);
    assert!(matches!(
        windows.window(old).unwrap().contents.contents,
        WidgetContents::Controls(_)
    ));
    assert!(windows.cancel_open(old));
    assert!(windows.window(old).is_none());
    assert!(windows.window_mut(old).is_none());
    let OpenRequest::Create(current) =
        windows.request_open(AuxKind::Settings, AuxKind::Settings.title())
    else {
        panic!()
    };
    assert_ne!(old, current);
    assert!(matches!(
        windows.window(current).unwrap().contents.contents,
        WidgetContents::PendingControls
    ));
    assert_eq!(windows.complete_open(old, Some(1), ()), Err(()));
    assert_eq!(windows.remove_id(old), None);
    assert!(!windows.cancel_open(old));
    assert!(windows.window(current).is_some());
    assert_eq!(windows.complete_open(current, Some(2), ()), Ok(()));
    assert!(!windows.cancel_open(current));
    assert!(windows.window(current).is_some());
    assert_eq!(windows.remove_id(current), Some(()));
    assert!(windows.window(current).is_none());
    let OpenRequest::Create(pending) = windows.request_open(AuxKind::Ppu, AuxKind::Ppu.title())
    else {
        panic!()
    };
    windows.clear();
    assert!(windows.window(pending).is_none());
    let OpenRequest::Create(fresh) = windows.request_open(AuxKind::Ppu, AuxKind::Ppu.title())
    else {
        panic!()
    };
    assert_ne!(fresh, pending);
    assert_eq!(windows.remove_id(pending), None);
    assert!(windows.window(fresh).is_some());
    assert_eq!(windows.remove(AuxKind::Ppu), None);
    assert!(windows.window(fresh).is_none());
}
