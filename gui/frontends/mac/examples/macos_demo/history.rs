//! Notification history window: level filter, count, clear, and a
//! datetime/level/module/message list, mirroring the gtk/osd dialog.

use nerust_gui_shell::session::notification::NotificationLevel;
use nerust_gui_shell::session::osd::{level_rgb, short_module};
use objc2::{MainThreadMarker, rc::Retained, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSButton, NSComboBox, NSTextField, NSTextView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

pub struct HistoryWin {
    pub window: Retained<NSWindow>,
    filter: Retained<NSComboBox>,
    count: Retained<NSTextField>,
    list: Retained<NSTextView>,
}

impl HistoryWin {
    pub fn open(
        mtm: MainThreadMarker,
        host: &mut nerust_mac::MacHost,
        delegate: &super::app::AppDelegate,
    ) -> Option<Self> {
        let frame = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(640.0, 440.0));
        let window: Retained<NSWindow> = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str("Notification History"));
        let filter: Retained<NSComboBox> = NSComboBox::initWithFrame(
            mtm.alloc::<NSComboBox>(),
            NSRect::new(NSPoint::new(8.0, 404.0), NSSize::new(160.0, 24.0)),
        );
        for name in ["All", "Info", "Warning", "Error"] {
            unsafe {
                filter.addItemWithObjectValue(&NSString::from_str(name));
            }
        }
        unsafe {
            filter.setTarget(Some(super::app::anyobject(delegate)));
        }
        unsafe {
            filter.setAction(Some(sel!(historyFilter:)));
        }
        if let Some(view) = window.contentView() {
            view.addSubview(&filter);
        }
        filter.selectItemAtIndex(0);
        let clear: Retained<NSButton> = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Clear"),
                Some(super::app::anyobject(delegate)),
                Some(sel!(historyClear:)),
                mtm,
            )
        };
        clear.setFrame(NSRect::new(
            NSPoint::new(176.0, 402.0),
            NSSize::new(90.0, 26.0),
        ));
        if let Some(view) = window.contentView() {
            view.addSubview(&clear);
        }
        let count: Retained<NSTextField> = NSTextField::initWithFrame(
            mtm.alloc::<NSTextField>(),
            NSRect::new(NSPoint::new(274.0, 404.0), NSSize::new(200.0, 20.0)),
        );
        count.setStringValue(&NSString::from_str(""));
        count.setEditable(false);
        count.setBezeled(false);
        count.setDrawsBackground(false);
        if let Some(view) = window.contentView() {
            view.addSubview(&count);
        }
        let list: Retained<NSTextView> = NSTextView::initWithFrame(
            mtm.alloc::<NSTextView>(),
            NSRect::new(NSPoint::new(8.0, 8.0), NSSize::new(624.0, 388.0)),
        );
        list.setEditable(false);
        if let Some(view) = window.contentView() {
            view.addSubview(&list);
        }
        window.makeKeyAndOrderFront(None);
        let mut win = Self {
            window,
            filter,
            count,
            list,
        };
        win.refresh(host);
        Some(win)
    }

    pub fn refresh(&mut self, host: &mut nerust_mac::MacHost) {
        let level = self.filter.indexOfSelectedItem().max(0) as usize;
        let language = host.session().settings_snapshot().shared.general.language;
        let lines = host
            .session_mut()
            .notification_history_lines_cached(language)
            .to_vec();
        let shown: Vec<_> = lines
            .iter()
            .filter(|line| {
                level == 0
                    || match line.level {
                        NotificationLevel::Info => level == 1,
                        NotificationLevel::Warning => level == 2,
                        NotificationLevel::Error => level == 3,
                    }
            })
            .collect();
        self.count
            .setStringValue(&NSString::from_str(&format!("{} items", shown.len())));
        let mut text = String::new();
        for line in shown {
            let (r, g, b) = level_rgb(line.level);
            let _ = (r, g, b);
            if line.count > 1 {
                text.push_str(&format!(
                    "{} [{}] {}: {} x{}\n",
                    line.datetime,
                    level_name(line.level),
                    short_module(line.module),
                    line.text,
                    line.count
                ));
            } else {
                text.push_str(&format!(
                    "{} [{}] {}: {}\n",
                    line.datetime,
                    level_name(line.level),
                    short_module(line.module),
                    line.text
                ));
            }
        }
        if text.is_empty() {
            text.push_str("(no notifications)");
        }
        self.list.setString(&NSString::from_str(&text));
    }
}

fn level_name(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "INFO",
        NotificationLevel::Warning => "WARN",
        NotificationLevel::Error => "ERROR",
    }
}
