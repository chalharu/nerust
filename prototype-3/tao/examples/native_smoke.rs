//! Run in its own process (GTK3 only), optionally under Xvfb.
use std::{cell::Cell, rc::Rc, time::Duration};
use wakui2_runtime::{Binding, Keyed, Layout, MenuItem, MenuRole, Store, Viewport};
use wakui2_tao::{View, run};

/// Application-defined advanced primitive, not a backend feature flag or string renderer.
struct ClipboardProbe(Rc<Cell<bool>>);
impl iced::advanced::Widget<wakui2_tao::Message, iced::Theme, iced_tiny_skia::Renderer>
    for ClipboardProbe
{
    fn size(&self) -> iced::Size<iced::Length> {
        iced::Size::new(iced::Length::Fixed(12.0), iced::Length::Fixed(12.0))
    }
    fn layout(
        &mut self,
        _: &mut iced::advanced::widget::Tree,
        _: &iced_tiny_skia::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        iced::advanced::layout::Node::new(limits.resolve(
            iced::Length::Fixed(12.0),
            iced::Length::Fixed(12.0),
            iced::Size::ZERO,
        ))
    }
    fn draw(
        &self,
        _: &iced::advanced::widget::Tree,
        renderer: &mut iced_tiny_skia::Renderer,
        _: &iced::Theme,
        _: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        _: iced::mouse::Cursor,
        _: &iced::Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        renderer.fill_quad(
            iced::advanced::renderer::Quad {
                bounds: layout.bounds(),
                ..Default::default()
            },
            iced::Color::from_rgb(0.1, 0.6, 0.3),
        );
    }
    fn update(
        &mut self,
        _: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        _: iced::advanced::Layout<'_>,
        _: iced::mouse::Cursor,
        _: &iced_tiny_skia::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        _: &mut iced::advanced::Shell<'_, wakui2_tao::Message>,
        _: &iced::Rectangle,
    ) {
        if !self.0.get()
            && matches!(
                event,
                iced::Event::Window(iced::window::Event::RedrawRequested(_))
            )
        {
            let previous = clipboard.read(iced::advanced::clipboard::Kind::Standard);
            clipboard.write(
                iced::advanced::clipboard::Kind::Standard,
                "wakui2 native clipboard probe".into(),
            );
            self.0.set(
                clipboard
                    .read(iced::advanced::clipboard::Kind::Standard)
                    .as_deref()
                    == Some("wakui2 native clipboard probe"),
            );
            if let Some(previous) = previous {
                clipboard.write(iced::advanced::clipboard::Kind::Standard, previous);
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn find_menu(widget: gtk::Widget, label: &str) -> Option<gtk::MenuItem> {
    use gtk::prelude::*;
    if let Some(item) = widget.downcast_ref::<gtk::MenuItem>() {
        if item.label().as_deref() == Some(label) {
            return Some(item.clone());
        }
        if let Some(submenu) = item.submenu()
            && let Some(item) = find_menu(submenu, label)
        {
            return Some(item);
        }
    }
    widget.downcast_ref::<gtk::Container>().and_then(|c| {
        c.children()
            .into_iter()
            .find_map(|child| find_menu(child, label))
    })
}

#[cfg(target_os = "linux")]
async fn native_menu_probe(invoked: &Cell<bool>) {
    use gtk::prelude::*;
    let window = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::Window>().ok())
        .find(|w| w.title().as_deref() == Some("native smoke"))
        .expect("native Tao toplevel");
    assert!(
        gtk::accel_groups_activate(
            &window,
            *gtk::gdk::keys::constants::s,
            gtk::gdk::ModifierType::CONTROL_MASK
        ),
        "muda shortcut is not installed on the actual native window"
    );
    let (send, receive) = futures::channel::oneshot::channel();
    gtk::glib::idle_add_local_once(move || {
        let _ = send.send(());
    });
    receive.await.unwrap();
    assert!(
        invoked.replace(false),
        "native hotkey did not reach the application dispatcher"
    );

    let about = find_menu(window.upcast(), "About").expect("actual native predefined About item");
    let observed = Rc::new(Cell::new(false));
    let out = observed.clone();
    // The predefined role enters GTK's modal loop; respond from its first idle.
    gtk::glib::idle_add_local_once(move || {
        for widget in gtk::Window::list_toplevels() {
            if let Ok(dialog) = widget.downcast::<gtk::AboutDialog>() {
                out.set(dialog.is_visible());
                dialog.response(gtk::ResponseType::Close);
            }
        }
    });
    about.emit_by_name::<()>("activate", &[]);
    assert!(
        observed.get(),
        "predefined About role did not show a real dialog"
    );
}

fn main() -> Result<(), String> {
    let closed = Rc::new(Cell::new(false));
    let observed = closed.clone();
    run(move |executor| {
        let model = Store::new("native smoke".to_owned(), executor.clone());
        let view = View::new(model.bind(Clone::clone, None));
        // A settings-like model controls readiness. Neither callback owns View.
        let close_state = Store::new((0usize, false), executor.clone());
        let weak = view.downgrade();
        view.keep(
            close_state
                .bind(|s| s.1, None)
                .subscribe(Rc::new(move |ready| {
                    if ready && let Some(view) = weak.upgrade() {
                        view.close();
                    }
                })),
        );
        let command = close_state.command(
            |_| true,
            |store, ()| {
                store.update(|s| {
                    s.0 += 1;
                    s.1 = s.0 == 3;
                });
            },
            Binding::constant(()),
        );
        view.on_close_request(Rc::new(move || {
            command.invoke();
        }));
        let panel = view.layout(
            &view.root(),
            "panel",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        view.entry(
            &panel,
            "entry",
            model.bind(Clone::clone, Some(|s, v| *s = v)),
        );
        view.meter(&panel, "meter", Binding::constant(0.7));
        let clipboard_ok = Rc::new(Cell::new(false));
        let probe = clipboard_ok.clone();
        view.mount(
            &panel,
            "application-primitive",
            Rc::new(move || wakui2_tao::Element::new(ClipboardProbe(probe.clone()))),
            Vec::new(),
        );
        view.items(
            &panel,
            "rows",
            Binding::constant(
                (0..100)
                    .map(|i| {
                        Keyed::new(
                            i.to_string(),
                            Store::new(format!("Row {i}"), executor.clone()),
                        )
                    })
                    .collect(),
            ),
            Binding::constant(Viewport {
                count: 4,
                row_height: 32.0,
                ..Viewport::default()
            }),
            Rc::new(|view, parent, _, row| {
                let panel = view.layout(parent, "row", Layout::Flex { gap: 4.0 });
                view.entry(&panel, "value", row.bind(Clone::clone, Some(|s, v| *s = v)));
            }),
        );
        let invoked = Rc::new(Cell::new(false));
        let out = invoked.clone();
        view.menu(
            "file",
            Binding::constant("File".into()),
            Binding::constant(vec![
                MenuItem {
                    id: "save".into(),
                    label: "Save".into(),
                    argument: "native argument".into(),
                    shortcut: Some("Primary+S".into()),
                    role: MenuRole::Normal,
                    enabled: true,
                },
                MenuItem {
                    id: "about".into(),
                    label: "About".into(),
                    argument: String::new(),
                    shortcut: None,
                    role: MenuRole::About,
                    enabled: true,
                },
                MenuItem {
                    id: "quit".into(),
                    label: "Quit".into(),
                    argument: "must not dispatch".into(),
                    shortcut: Some("Primary+Q".into()),
                    role: MenuRole::Quit,
                    enabled: true,
                },
            ]),
            Rc::new(move |arg| {
                assert_ne!(arg, "must not dispatch");
                out.set(arg == "native argument");
                true
            }),
        );
        let owner = model.clone();
        let state = close_state.clone();
        view.on_close(Rc::new(move || {
            assert_eq!(
                state.snapshot(),
                (3, true),
                "WM/menu/shortcut requested close"
            );
            state.close();
            owner.close();
            observed.set(true);
        }));
        let (send, receive) = futures::channel::oneshot::channel();
        // A single worker completion, not a UI polling timer. Leave time for actual drawing.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(800));
            let _ = send.send(());
        });
        let out = view.clone();
        executor.spawn(Box::pin(async move {
            receive.await.unwrap();
            assert!(out.frame_count() > 0, "native window never drew");
            assert!(
                clipboard_ok.get(),
                "custom advanced primitive could not roundtrip the system clipboard"
            );
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::*;
                let window = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Window>().ok())
                    .find(|w| w.title().as_deref() == Some("native smoke"))
                    .unwrap();
                window.close(); // Real GTK WM delete-event through Tao CloseRequested.
                let (send, receive) = futures::channel::oneshot::channel();
                gtk::glib::idle_add_local_once(move || {
                    let _ = send.send(());
                });
                receive.await.unwrap();
                assert_eq!(close_state.snapshot(), (1, false));
                assert!(window.is_visible(), "first request was vetoed");
                native_menu_probe(&invoked).await;
            }
            #[cfg(not(target_os = "linux"))]
            out.request_close(); // Native WM delivery on these platforms is unverified.
            out.edit("entry", "worker woke UI").unwrap();
            out.viewport("rows", 80);
            out.activate_menu("file/save").unwrap();
            assert!(invoked.get());
            assert_eq!(out.item_keys("rows"), ["80", "81", "82", "83"]);
            assert!(out.diagnostics().is_empty(), "{:?}", out.diagnostics());
            println!(
                "native-smoke: frames={}, rows={}, worker wake/menu/entry/clipboard OK",
                out.frame_count(),
                out.mounted_count("rows")
            );
            #[cfg(target_os = "linux")]
            {
                use gtk::prelude::*;
                let quit = gtk::Window::list_toplevels()
                    .into_iter()
                    .find_map(|window| find_menu(window, "Quit"))
                    .expect("actual native Linux Quit shim");
                assert!(quit.is_sensitive());
                quit.emit_by_name::<()>("activate", &[]);
                let (send, receive) = futures::channel::oneshot::channel();
                gtk::glib::idle_add_local_once(move || {
                    let _ = send.send(());
                });
                receive.await.unwrap();
                assert_eq!(close_state.snapshot(), (2, false));
                let title = out.title();
                let window = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::Window>().ok())
                    .find(|w| w.title().as_deref() == Some(title.as_str()))
                    .unwrap();
                assert!(gtk::accel_groups_activate(
                    &window,
                    *gtk::gdk::keys::constants::q,
                    gtk::gdk::ModifierType::CONTROL_MASK
                ));
                // Native event delivery is asynchronous. Close cancels UI tasks, so
                // assert from on_close rather than awaiting another scheduler turn.
                assert!(!model.is_closed());
            }
            #[cfg(not(target_os = "linux"))]
            {
                out.activate_menu("file/quit").unwrap();
                out.activate_menu("file/quit").unwrap();
            }
        }));
        view
    })?;
    assert!(closed.get(), "native Quit must close the owning View");
    println!("native-smoke: close callback OK (Linux native WM/menu/Primary+Q requests)");
    Ok(())
}
