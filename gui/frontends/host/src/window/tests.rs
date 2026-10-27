use std::{cell::RefCell, sync::Arc};

use super::*;
use crate::{
    aux::{AuxWindows, OpenRequest},
    settings::{
        SettingsControl, SettingsControlId, SettingsControlKind, SettingsController, SettingsEvent,
        SettingsScreenPage, SettingsVisitor,
    },
};

fn controller() -> SettingsController {
    SettingsController::new(
        &nerust_gui_runtime::settings::SettingsSnapshot {
            shared: Default::default(),
            local: Default::default(),
            app_state: Default::default(),
        },
        Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![])),
        Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
    )
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
        fn visit_debug(&mut self, _: &DebugContents) {
            self.0.push("debug");
        }
        fn visit_ppu(&mut self, _: &PpuContents) {
            self.0.push("ppu");
        }
        fn visit_history(&mut self, _: &HistoryContents) {
            self.0.push("history");
        }
        fn visit_pending_settings(&mut self) {
            self.0.push("pending");
        }
    }
    for (window, group) in [
        (Window::main(), "main"),
        (Window::auxiliary(AuxKind::Debug), "debug"),
        (Window::auxiliary(AuxKind::Ppu), "ppu"),
        (Window::auxiliary(AuxKind::History), "history"),
        (Window::auxiliary(AuxKind::Settings), "pending"),
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
        controller: &'a RefCell<SettingsController>,
        pages: usize,
        controls: usize,
    }
    impl WindowVisitor for Visitor<'_> {
        fn visit_settings(&mut self, screen: &SettingsScreen) {
            self.controller
                .borrow_mut()
                .update(SettingsEvent::SetVolume(19));
            let nested = self.controller.borrow().window();
            assert!(matches!(
                nested.contents.contents,
                WidgetContents::Settings(_)
            ));
            let dynamic: &mut dyn SettingsVisitor = self;
            screen.visit(dynamic);
        }
    }
    impl SettingsVisitor for Visitor<'_> {
        fn begin_page(&mut self, _: &SettingsScreenPage) {
            self.pages += 1;
        }
        fn control(&mut self, _: &SettingsControl) {
            self.controls += 1;
        }
    }
    let controller = RefCell::new(controller());
    controller.borrow_mut().update(SettingsEvent::SetVolume(37));
    controller
        .borrow()
        .vm
        .general
        .set_language(nerust_settings_core::model::language::AppLanguage::Japanese)
        .unwrap();
    let revision = controller.borrow().vm.revision.get();
    let window = controller.borrow().window();
    assert_eq!(controller.borrow().vm.revision.get(), revision);
    assert_eq!(window.id, WindowId::Auxiliary(AuxKind::Settings));
    let WidgetContents::Settings(screen) = &window.contents.contents else {
        panic!("settings tree must be owned by the widget");
    };
    assert_eq!(window.title, screen.title);
    let mut visitor = Visitor {
        controller: &controller,
        pages: 0,
        controls: 0,
    };
    window.visit(&mut visitor as &mut dyn WindowVisitor);
    assert_eq!(visitor.pages, 6);
    assert!(visitor.controls > 20);
    assert!(matches!(
        screen.control(&SettingsControlId::Volume).unwrap().kind,
        SettingsControlKind::Number { value: 37, .. }
    ));
    assert_eq!(controller.borrow().vm.audio.view.get().volume_percent, 19);
    let latest = controller.borrow().window();
    drop(controller);
    let WidgetContents::Settings(screen) = latest.contents.contents else {
        panic!()
    };
    assert!(matches!(
        screen.control(&SettingsControlId::Volume).unwrap().kind,
        SettingsControlKind::Number { value: 19, .. }
    ));
}

#[test]
fn reservation_owns_group_before_native_creation_and_reentrant_open() {
    let windows = RefCell::new(AuxWindows::<u64, ()>::new());
    for kind in [AuxKind::Debug, AuxKind::Ppu] {
        let OpenRequest::Create(id) = windows.borrow_mut().request_open(kind) else {
            panic!()
        };
        let snapshot = windows.borrow().window(id).unwrap().clone();
        assert_eq!(snapshot.id, WindowId::Auxiliary(kind));
        assert!(matches!(
            (kind, &snapshot.contents.contents),
            (AuxKind::Debug, WidgetContents::Debug(_)) | (AuxKind::Ppu, WidgetContents::Ppu(_))
        ));
        struct Reenter<'a> {
            windows: &'a RefCell<AuxWindows<u64, ()>>,
            id: crate::aux::AuxId,
        }
        impl WindowVisitor for Reenter<'_> {
            fn begin_window(&mut self, window: &Window) {
                let WindowId::Auxiliary(kind) = window.id else {
                    panic!()
                };
                assert_eq!(
                    self.windows.borrow_mut().request_open(kind),
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
            windows.borrow_mut().request_open(kind),
            OpenRequest::Present(id)
        );
    }
}

#[test]
fn blueprint_lifetime_matches_reservation_retry_removal_and_clear() {
    let mut windows = AuxWindows::<u64, ()>::new();
    let OpenRequest::Create(old) = windows.request_open(AuxKind::Settings) else {
        panic!()
    };
    *windows.window_mut(old).unwrap() = controller().window();
    assert!(matches!(
        windows.window(old).unwrap().contents.contents,
        WidgetContents::Settings(_)
    ));
    assert!(windows.cancel_open(old));
    assert!(windows.window(old).is_none());
    assert!(windows.window_mut(old).is_none());
    let OpenRequest::Create(current) = windows.request_open(AuxKind::Settings) else {
        panic!()
    };
    assert_ne!(old, current);
    assert!(matches!(
        windows.window(current).unwrap().contents.contents,
        WidgetContents::PendingSettings
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
    let OpenRequest::Create(pending) = windows.request_open(AuxKind::Ppu) else {
        panic!()
    };
    windows.clear();
    assert!(windows.window(pending).is_none());
    let OpenRequest::Create(fresh) = windows.request_open(AuxKind::Ppu) else {
        panic!()
    };
    assert_ne!(fresh, pending);
    assert_eq!(windows.remove_id(pending), None);
    assert!(windows.window(fresh).is_some());
    assert_eq!(windows.remove(AuxKind::Ppu), None);
    assert!(windows.window(fresh).is_none());
}
