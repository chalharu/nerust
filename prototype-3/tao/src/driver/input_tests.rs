//! Isolated GTK3/Tao/X11 regressions. Never load GTK4 in this process.
use super::*;
use iced::advanced::Clipboard as _;
use std::time::Duration;
use tao::platform::unix::EventLoopBuilderExtUnix;
use wakui2_runtime::{Binding, Store};

// Only the GTK-independent, dynamically loaded XTest helper is shared.
#[path = "../../../gtk/src/x11_test.rs"]
mod x11;

struct NativeInput {
    events: tao::event_loop::EventLoop<Native>,
    desktop: Desktop,
}
impl NativeInput {
    fn new(view: &View) -> Self {
        // libtest runs this isolated test on its own thread.
        let mut builder = EventLoopBuilder::<Native>::with_user_event();
        builder.with_any_thread(true);
        let events = builder.build();
        let desktop = Desktop::open(&events, view, Proxy(events.create_proxy())).unwrap();
        view.present();
        desktop.refresh();
        let mut input = Self { events, desktop };
        input.turn();
        input
    }
    fn turn(&mut self) {
        // Bounded real native loop: process GDK keys, Tao events and GTK drawing.
        // Nonblocking native turns avoid Tao's GTK3 blocking-iteration wake
        // limitation. Assertions below check actual model progress.
        let until = Instant::now() + Duration::from_millis(60);
        self.events.run_return(|event, _, flow| {
            if let Event::WindowEvent {
                window_id, event, ..
            } = event
                && window_id == self.desktop.window.id()
            {
                self.desktop.event(&event);
            }
            *flow = if Instant::now() >= until {
                ControlFlow::Exit
            } else {
                ControlFlow::Poll
            };
        });
        assert!(
            self.desktop.view.error().is_none(),
            "{:?}",
            self.desktop.view.diagnostics()
        );
    }
    fn focus_window(&mut self, keys: &x11::X11) {
        let native = self.desktop.window.gtk_window();
        assert_eq!(native.display().type_().name(), "GdkX11Display");
        let window = native.window().unwrap();
        unsafe extern "C" {
            fn gdk_x11_window_get_xid(window: *mut gtk::gdk::ffi::GdkWindow) -> std::ffi::c_ulong;
        }
        // SAFETY: checked X11 backend and live realized window.
        keys.focus(unsafe { gdk_x11_window_get_xid(window.as_ptr()) });
        self.desktop.area.grab_focus();
        self.turn();
        assert!(self.desktop.area.has_focus());
    }
    fn click_field(&mut self, index: usize) {
        // Explicit backend-event mouse injection, while keyboard input below is
        // delivered by XTest through the actual GTK3/Tao native key callbacks.
        let offset = self.desktop.offset();
        let scale = self.desktop.window.scale_factor();
        #[allow(deprecated)]
        self.desktop.event(&WindowEvent::CursorMoved {
            device_id: unsafe { tao::event::DeviceId::dummy() },
            position: tao::dpi::PhysicalPosition::new(
                (offset.x as f64 + 10.0) * scale,
                (offset.y as f64 + 15.0 + index as f64 * 40.0) * scale,
            ),
            modifiers: tao::keyboard::ModifiersState::empty(),
        });
        for state in [
            tao::event::ElementState::Pressed,
            tao::event::ElementState::Released,
        ] {
            #[allow(deprecated)]
            self.desktop.event(&WindowEvent::MouseInput {
                device_id: unsafe { tao::event::DeviceId::dummy() },
                state,
                button: tao::event::MouseButton::Left,
                modifiers: tao::keyboard::ModifiersState::empty(),
            });
        }
        self.turn();
    }
}

fn fields() -> (View, Store<(String, String, usize)>, Store<u32>) {
    let executor = Rc::new(Scheduler::new(Arc::new(|_| {})));
    let model = Store::new((String::new(), String::new(), 0usize), executor.clone());
    let number = Store::new(12u32, executor);
    let view = View::new(Binding::constant("native Tao input".into()));
    let parent = view.layout(&view.root(), "fields", wakui2_runtime::Layout::Absolute);
    view.entry(
        &parent,
        "first",
        model.bind(
            |s| s.0.clone(),
            Some(|s, v| {
                s.0 = v;
                s.2 += 1;
            }),
        ),
    );
    view.entry(
        &parent,
        "second",
        model.bind(
            |s| s.1.clone(),
            Some(|s, v| {
                s.1 = v;
                s.2 += 1;
            }),
        ),
    );
    view.entry(&parent, "readonly", Binding::constant("読取専用".into()));
    view.entry(
        &parent,
        "number",
        number.bind(|s| *s, Some(|s, v| *s = v)).convert(
            |v| v.to_string(),
            |v: String| v.parse::<u32>().map_err(|e| e.to_string()),
        ),
    );
    for (index, id) in ["first", "second", "readonly", "number"]
        .into_iter()
        .enumerate()
    {
        view.style(
            id,
            wakui2_runtime::Style {
                placement: wakui2_runtime::Placement {
                    y: index as f32 * 40.0,
                    width: Some(300.0),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }
    (view, model, number)
}

#[test]
#[ignore = "isolated Xvfb, GTK_IM_MODULE=simple; optional libXtst (WAKUI_REQUIRE_XTEST=1 makes absence fail)"]
fn native_x11_keyboard_ime_signal_iced_fields_and_clipboard() {
    let Some(mut keys) = x11::X11::optional() else {
        return;
    };
    assert_eq!(std::env::var("GTK_IM_MODULE").as_deref(), Ok("simple"));
    let (view, model, number) = fields();
    let mut input = NativeInput::new(&view);
    input.focus_window(&keys);
    input.click_field(0);
    keys.tap(b'a'.into());
    input.turn();
    assert_eq!(
        model.snapshot(),
        ("a".into(), "".into(), 1),
        "physical key text is delivered exactly once, not as both key text and IM commit"
    );
    keys.chord(&[0xffe3], b'a'.into());
    input.turn();

    // Explicit native commit SIGNAL injection, not external IME/preedit or a
    // claim that Tao's internal IMContextSimple supports Unicode compose.
    // Tao 0.37 filters only key-press, not key-release; Ctrl+Shift+U currently
    // produces literal hex digits here. GTK4's separate test covers real compose.
    let ime = gtk::IMContextSimple::new();
    ime.set_client_window(input.desktop.window.gtk_window().window().as_ref());
    ime.focus_in();
    let commits = Rc::new(RefCell::new(Vec::new()));
    let output = commits.clone();
    let signal = ime.connect_commit(move |_, text| output.borrow_mut().push(text.to_owned()));
    let commit = |input: &mut NativeInput, text: &str| {
        ime.emit_by_name::<()>("commit", &[&text]);
        let output = commits.take();
        assert_eq!(output, [text]);
        for text in output {
            input.desktop.event(&WindowEvent::ReceivedImeText(text));
        }
    };
    commit(&mut input, "日本語");
    assert_eq!(
        model.snapshot(),
        ("日本語".into(), "".into(), 2),
        "multi-codepoint commit replaces selection through the real iced field once"
    );
    assert_eq!(view.text("first").as_deref(), Some("日本語"));
    keys.chord(&[0xffe3], b'a'.into());
    keys.chord(&[0xffe3], b'c'.into());
    input.turn();
    assert_eq!(
        input
            .desktop
            .painter
            .borrow()
            .clipboard
            .read(iced::advanced::clipboard::Kind::Standard)
            .as_deref(),
        Some("日本語")
    );
    input.click_field(1);
    keys.chord(&[0xffe3], b'v'.into());
    input.turn();
    assert_eq!(
        model.snapshot().1,
        "日本語",
        "native clipboard paste edits second field"
    );
    keys.tap(0xff08);
    input.turn();
    assert_eq!(model.snapshot().1, "日本");
    keys.chord(&[0xffe1], 0xff51);
    keys.tap(0xffff);
    input.turn();
    assert_eq!(
        model.snapshot().1,
        "日",
        "Shift+Left/Delete removes selected Unicode grapheme"
    );
    assert_eq!(model.snapshot().0, "日本語");
    input.click_field(2);
    commit(&mut input, "無効");
    keys.chord(&[0xffe3], b'v'.into());
    keys.tap(0xff08);
    input.turn();
    assert_eq!(view.text("readonly").as_deref(), Some("読取専用"));
    assert_eq!(model.snapshot().1, "日");
    input.click_field(3);
    keys.chord(&[0xffe3], b'a'.into());
    keys.chord(&[0xffe3], b'v'.into());
    input.turn();
    assert_eq!(number.snapshot(), 12);
    assert_eq!(
        view.text("number").as_deref(),
        Some("12"),
        "rejected Japanese paste must not leave a false cached value"
    );
    ime.focus_out();
    ime.disconnect(signal);
    ime.set_client_window(None::<&gtk::gdk::Window>);
    view.close();
}

#[test]
#[ignore = "requires isolated Xvfb GTK3 process; backend-event injection into actual iced UI"]
fn native_backend_focus_loss_clears_iced_modifiers() {
    let (view, model, _) = fields();
    let mut input = NativeInput::new(&view);
    input.click_field(0);
    // Not physical X11 input: deliberately omit the modifier-release event to
    // reproduce switching windows while Ctrl/Shift are held.
    input
        .desktop
        .event(&WindowEvent::ReceivedImeText("日本語".into()));
    input.desktop.event(&WindowEvent::ModifiersChanged(
        tao::keyboard::ModifiersState::CONTROL | tao::keyboard::ModifiersState::SHIFT,
    ));
    input.desktop.event(&WindowEvent::Focused(false));
    assert!(input.desktop.painter.borrow().modifiers.is_empty());
    input.desktop.event(&WindowEvent::Focused(true));
    // ArrowLeft must move one character (not select/jump a word), then commit
    // inserts at that caret rather than replacing a stale Shift selection.
    let left = iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft),
        modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowLeft),
        physical_key: iced::keyboard::key::Physical::Unidentified(
            iced::keyboard::key::NativeCode::Unidentified,
        ),
        location: iced::keyboard::Location::Standard,
        modifiers: iced::keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    });
    let messages =
        input
            .desktop
            .painter
            .borrow_mut()
            .pump(&view, input.desktop.bounds(), left, false);
    dispatch(&view, messages);
    input
        .desktop
        .event(&WindowEvent::ReceivedImeText("あ".into()));
    assert_eq!(
        model.snapshot().0,
        "日本あ語",
        "iced widget modifiers, not only converter modifiers, must reset on focus loss"
    );
    assert_eq!(view.text("first").as_deref(), Some("日本あ語"));
    view.close();
}
