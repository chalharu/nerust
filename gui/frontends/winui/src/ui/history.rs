//! Notification history window: level filter, count, clear, and a
//! datetime/level/module/message grid, mirroring the gtk/osd dialog.

use nerust_gui_shell::session::notification::NotificationLevel;
use nerust_gui_shell::session::osd::{level_rgb, short_module};
use windows_core::Result;

use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button, ComboBox, ListView, Orientation, SelectionChangedEventHandler, StackPanel, TextBlock,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use crate::bindings::Microsoft::UI::Xaml::{RoutedEventHandler, Window};
use crate::bindings::Windows::UI::Color;

use super::{box_str, h};

const FILTERS: [&str; 4] = ["All", "Info", "Warning", "Error"];

pub struct HistoryWin {
    pub window: Window,
    filter: ComboBox,
    count: TextBlock,
    list: ListView,
    level: usize,
}

impl HistoryWin {
    pub fn open(host: &mut nerust_win::WinHost) -> Option<Self> {
        let flow = (|| -> Result<Self> {
            let window = Window::new()?;
            window.SetTitle(&h("Notification History"))?;
            let filter = ComboBox::new()?;
            {
                let items = filter.Items()?;
                for name in FILTERS {
                    items.Append(&box_str(name)?)?;
                }
            }
            filter.SetSelectedIndex(0)?;
            filter.SelectionChanged(&SelectionChangedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.refresh_history());
                Ok(())
            }))?;
            let clear = Button::new()?;
            clear.SetContent(&box_str("Clear")?)?;
            clear.Click(&RoutedEventHandler::new(|_, _| {
                super::app::with_app(|app| app.clear_history());
                Ok(())
            }))?;
            let count = TextBlock::new()?;
            let toolbar = StackPanel::new()?;
            toolbar.SetOrientation(Orientation::Horizontal)?;
            toolbar.Children()?.Append(&filter)?;
            toolbar.Children()?.Append(&clear)?;
            toolbar.Children()?.Append(&count)?;
            let list = ListView::new()?;
            let stack = StackPanel::new()?;
            stack.SetOrientation(Orientation::Vertical)?;
            stack.Children()?.Append(&toolbar)?;
            stack.Children()?.Append(&list)?;
            window.SetContent(&stack)?;
            window.Activate()?;
            let mut win = Self {
                window,
                filter,
                count,
                list,
                level: 0,
            };
            win.refresh(host);
            Ok(win)
        })();
        match flow {
            Ok(win) => Some(win),
            Err(error) => {
                log::error!("winui history open failed: {error}");
                None
            }
        }
    }

    pub fn refresh(&mut self, host: &mut nerust_win::WinHost) {
        let level = self.filter.SelectedIndex().unwrap_or(0).max(0) as usize;
        self.level = level;
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
        let _ = self.count.SetText(&h(&format!("{} items", shown.len())));
        if self.list.Items().and_then(|items| items.Clear()).is_ok() {
            for line in shown {
                let (r, g, b) = level_rgb(line.level);
                let text = if line.count > 1 {
                    format!(
                        "{} [{}] {}: {} x{}",
                        line.datetime,
                        level_name(line.level),
                        short_module(line.module),
                        line.text,
                        line.count
                    )
                } else {
                    format!(
                        "{} [{}] {}: {}",
                        line.datetime,
                        level_name(line.level),
                        short_module(line.module),
                        line.text
                    )
                };
                let Ok(row) = TextBlock::new() else {
                    break;
                };
                if row.SetText(&h(&text)).is_err() {
                    break;
                }
                let Ok(brush) = SolidColorBrush::new() else {
                    break;
                };
                if brush
                    .SetColor(Color {
                        A: 255,
                        R: r,
                        G: g,
                        B: b,
                    })
                    .is_err()
                    || row.SetForeground(&brush).is_err()
                {
                    break;
                }
                if let Ok(items) = self.list.Items()
                    && let Ok(item) = box_str(&text)
                    && items.Append(&item).is_err()
                {
                    let _ = item;
                    break;
                }
            }
        }
    }
}

fn level_name(level: NotificationLevel) -> &'static str {
    match level {
        NotificationLevel::Info => "INFO",
        NotificationLevel::Warning => "WARN",
        NotificationLevel::Error => "ERROR",
    }
}
