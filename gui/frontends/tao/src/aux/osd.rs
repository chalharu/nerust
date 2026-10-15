//! OSD notifications: display queue, transparent overlay window,
//! and history viewer.
//!
//! Shell owns event capture (`NotificationCenter`); this module owns
//! presentation state. `OsdManager` is display-free so headless tests
//! cover queueing, expiry, burst caps, and strip math. The overlay
//! window itself (`OsdWindowHandle`) is a transparent click-through
//! iced shell tracking the main client area.

use std::time::{Duration, Instant};

use iced::{Length, Task};
use iced_winit::program::{self, Program};

use nerust_gui_shell::session::notification::{NotificationLevel, UiNotification};
use nerust_settings_core::model::{
    language::AppLanguage,
    local::{NotificationAreaMode, NotificationPosition, NotificationSettings},
};
use tao::event_loop::EventLoopWindowTarget;

use crate::aux::shell::{AuxWindow, WindowScaffold, aux_settings, aux_theme};

/// Glyph per level: shape (not color alone) distinguishes severity.
fn level_glyph(level: NotificationLevel) -> char {
    match level {
        NotificationLevel::Info => 'i',
        NotificationLevel::Warning => '!',
        NotificationLevel::Error => '×',
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OsdRow {
    pub(crate) text: String,
    pub(crate) rgb: (u8, u8, u8),
    pub(crate) glyph: char,
}

struct OsdItem {
    row: OsdRow,
    /// `None` holds indefinitely (Error-as-OSD without successor).
    deadline: Option<Instant>,
    /// Earliest replacement time (Error-as-OSD holds 5s minimum).
    replaceable_at: Instant,
}

pub(crate) struct OsdManager {
    items: Vec<OsdItem>,
    modal: Vec<String>,
    error_hold: Option<OsdItem>,
}

impl OsdManager {
    pub(crate) fn new() -> Self {
        Self {
            items: Vec::new(),
            modal: Vec::new(),
            error_hold: None,
        }
    }

    fn enabled(level: NotificationLevel, settings: &NotificationSettings) -> bool {
        match level {
            NotificationLevel::Info => settings.info_enabled,
            NotificationLevel::Warning => settings.warning_enabled,
            NotificationLevel::Error => settings.error_enabled,
        }
    }

    fn duration_ms(level: NotificationLevel, settings: &NotificationSettings) -> u64 {
        let raw = match level {
            NotificationLevel::Info => settings.info_duration_ms,
            NotificationLevel::Warning => settings.warning_duration_ms,
            NotificationLevel::Error => 5_000,
        };
        u64::from(raw.clamp(500, 60_000))
    }

    fn color(level: NotificationLevel, settings: &NotificationSettings) -> (u8, u8, u8) {
        match level {
            NotificationLevel::Info => settings.info_color.rgb(),
            NotificationLevel::Warning => settings.warning_color.rgb(),
            NotificationLevel::Error => settings.error_color.rgb(),
        }
    }

    /// Fold drained shell notifications into display state.
    /// Errors go modal (aggregated) or OSD-hold per settings.
    pub(crate) fn push_all(
        &mut self,
        notifications: Vec<UiNotification>,
        settings: &NotificationSettings,
        language: AppLanguage,
        now: Instant,
    ) {
        for n in notifications {
            if !Self::enabled(n.level, settings) {
                continue;
            }
            let mut text = n.kind.text(language);
            if n.count > 1 {
                text = format!("{text} ×{}", n.count);
            }
            let row = OsdRow {
                text,
                rgb: Self::color(n.level, settings),
                glyph: level_glyph(n.level),
            };
            if n.level == NotificationLevel::Error {
                if settings.error_modal {
                    self.modal.push(row.text);
                    continue;
                }
                // Error hold: minimum 5s, replaced by the next error
                // once mature. An immature hold keeps its slot.
                let mature = self
                    .error_hold
                    .as_ref()
                    .is_none_or(|hold| hold.replaceable_at <= now);
                if mature {
                    self.error_hold = Some(OsdItem {
                        row,
                        deadline: None,
                        replaceable_at: now + Duration::from_secs(5),
                    });
                } else {
                    self.push_transient(
                        OsdItem {
                            row,
                            deadline: Some(
                                now + Duration::from_millis(Self::duration_ms(
                                    NotificationLevel::Error,
                                    settings,
                                )),
                            ),
                            replaceable_at: now,
                        },
                        now,
                    );
                }
                continue;
            }
            self.push_transient(
                OsdItem {
                    row,
                    deadline: Some(
                        now + Duration::from_millis(Self::duration_ms(n.level, settings)),
                    ),
                    replaceable_at: now,
                },
                now,
            );
        }
    }

    fn push_transient(&mut self, item: OsdItem, now: Instant) {
        // Error hold yields to fresh transients once mature.
        if let Some(hold) = self.error_hold.take() {
            if hold.replaceable_at <= now {
                drop(hold);
            } else {
                self.error_hold = Some(hold);
            }
        }
        if self.items.len() >= nerust_gui_shell::session::notification::MAX_PENDING {
            self.items.remove(0);
        }
        self.items.push(item);
    }

    /// Drop expired items. Returns true when the view changed.
    pub(crate) fn poll(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items
            .retain(|item| item.deadline.is_none_or(|deadline| deadline > now));
        before != self.items.len()
    }

    pub(crate) fn rows(&self) -> Vec<OsdRow> {
        let mut rows: Vec<OsdRow> = self.items.iter().map(|item| item.row.clone()).collect();
        if let Some(hold) = self.error_hold.as_ref() {
            rows.push(hold.row.clone());
        }
        rows
    }

    pub(crate) fn take_modal(&mut self) -> Vec<String> {
        std::mem::take(&mut self.modal)
    }

    pub(crate) fn has_visible(&self) -> bool {
        !self.items.is_empty() || self.error_hold.is_some()
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.items.iter().filter_map(|item| item.deadline).min()
    }

    pub(crate) fn status_text(fps: f32, paused: bool, language: AppLanguage) -> String {
        if paused {
            match language {
                AppLanguage::Japanese => format!("一時停止中 {fps:.1} fps"),
                _ => format!("Paused {fps:.1} fps"),
            }
        } else {
            format!("{fps:.1} fps")
        }
    }
}

/// Strip rows per edge from transient + status corners.
/// Same edge, different sides share one strip; same corner stacks two.
pub(crate) fn strip_rows(
    area_mode: NotificationAreaMode,
    transient: NotificationPosition,
    status: NotificationPosition,
    status_on: bool,
) -> (u32, u32) {
    if !matches!(area_mode, NotificationAreaMode::Strip) {
        return (0, 0);
    }
    fn edge(position: NotificationPosition) -> bool {
        matches!(
            position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        )
    }
    let mut top = [0u32, 0u32];
    let mut bottom = [0u32, 0u32];
    let sides = [(transient, true), (status, status_on)];
    for (position, on) in sides {
        if !on {
            continue;
        }
        let side = usize::from(matches!(
            position,
            NotificationPosition::TopRight | NotificationPosition::BottomRight
        ));
        if edge(position) {
            top[side] += 1;
        } else {
            bottom[side] += 1;
        }
    }
    (top[0].max(top[1]), bottom[0].max(bottom[1]))
}

/// Physical strip pixels from the client height. Label rows scale
/// with the viewport (2.2%); padding is fixed.
pub(crate) fn strip_pixels(client_height: u32, rows: u32) -> u32 {
    if rows == 0 || client_height == 0 {
        return 0;
    }
    let font_px = (f64::from(client_height) * 0.022).ceil() as u32;
    rows.saturating_mul(font_px.saturating_add(8))
}

// ── Overlay window ───────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub(crate) struct OsdSnapshot {
    pub(crate) rows: Vec<OsdRow>,
    pub(crate) status: Option<String>,
    pub(crate) position: NotificationPosition,
    pub(crate) status_position: NotificationPosition,
    pub(crate) font_px: f32,
}

pub(crate) struct OsdState {
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

pub(crate) struct OsdProgram {
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

impl Program for OsdProgram {
    type State = OsdState;
    type Message = ();
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_osd"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            OsdState {
                shared: std::sync::Arc::clone(&self.shared),
            },
            Task::none(),
        )
    }

    fn update(&self, _state: &mut Self::State, _message: Self::Message) -> Task<Self::Message> {
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, container, row, text};
        let snapshot = state.shared.lock().map(|s| s.clone()).unwrap_or_default();
        let font = iced::Pixels(snapshot.font_px.max(6.0));
        let label = |glyph: char, body: &str, rgb: (u8, u8, u8)| {
            container(
                row![
                    text(format!("{glyph} ")).size(font),
                    text(body.to_string()).size(font),
                ]
                .spacing(4),
            )
            .padding([2, 6])
            .style(move |_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba8(
                    0, 0, 0, 0.72,
                ))),
                text_color: Some(iced::Color::from_rgb8(rgb.0, rgb.1, rgb.2)),
                border: iced::Border {
                    radius: 6.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
        };
        // Transient rows anchor at `position`, status at
        // `status_position`: same-corner stacking and split-corner
        // placement both fall out of one two-group column.
        let top_is_transient = matches!(
            snapshot.position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        );
        let status_top = matches!(
            snapshot.status_position,
            NotificationPosition::TopLeft | NotificationPosition::TopRight
        );
        let transient: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> = {
            let mut items = column![].spacing(4);
            for row_data in &snapshot.rows {
                items = items.push(label(row_data.glyph, &row_data.text, row_data.rgb));
            }
            container(items)
                .align_x(match snapshot.position {
                    NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                        iced::alignment::Horizontal::Left
                    }
                    NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                        iced::alignment::Horizontal::Right
                    }
                })
                .into()
        };
        let status_widget: iced::Element<'_, (), iced::Theme, iced_tiny_skia::Renderer> =
            match snapshot.status.as_ref() {
                Some(status) => container(label('\u{25cf}', status, (200, 200, 200)))
                    .align_x(match snapshot.status_position {
                        NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                            iced::alignment::Horizontal::Left
                        }
                        NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                            iced::alignment::Horizontal::Right
                        }
                    })
                    .into(),
                None => iced::widget::Space::new().into(),
            };
        // Same edge: status pins to the edge side, transient inside.
        let (top, bottom) = if top_is_transient == status_top {
            if top_is_transient {
                (status_widget, transient)
            } else {
                (transient, status_widget)
            }
        } else if top_is_transient {
            (transient, status_widget)
        } else {
            (status_widget, transient)
        };
        container(
            column![
                top,
                iced::widget::Space::new()
                    .width(Length::Fill)
                    .height(Length::Fill),
                bottom,
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(8)
        .into()
    }
}

pub(crate) struct OsdWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<OsdProgram>,
    shared: std::sync::Arc<std::sync::Mutex<OsdSnapshot>>,
}

impl OsdWindowHandle {
    /// Transparent click-through overlay tracking the main client area.
    /// Returns `None` when the platform refuses transparency (OSD off).
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        position: (i32, i32),
        size: (f64, f64),
    ) -> Option<Self> {
        use tao::window::WindowBuilder;
        let window = WindowBuilder::new()
            .with_title("nerust OSD")
            .with_inner_size(tao::dpi::LogicalSize::new(size.0, size.1))
            .with_position(tao::dpi::LogicalPosition::new(
                position.0 as f64,
                position.1 as f64,
            ))
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top(true)
            .build(event_loop)
            .map_err(|error| {
                log::warn!("OSD overlay unavailable: {error}");
            })
            .ok()?;
        if let Err(error) = window.set_ignore_cursor_events(true) {
            log::warn!("OSD click-through refused: {error:?}");
            return None;
        }
        let window = std::sync::Arc::new(window);
        let scaffold = WindowScaffold::open_transparent(window, "OSD overlay")?;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(OsdSnapshot::default()));
        let (instance, _task) = program::Instance::new(OsdProgram {
            shared: std::sync::Arc::clone(&shared),
        });
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self { shell, shared })
    }

    pub(crate) fn sync(&mut self, snapshot: OsdSnapshot) {
        if let Ok(mut shared) = self.shared.lock() {
            *shared = snapshot;
        }
        self.shell.window.request_redraw();
    }

    pub(crate) fn move_to(&mut self, position: (i32, i32), size: (f64, f64)) {
        self.shell
            .window
            .set_outer_position(tao::dpi::LogicalPosition::new(
                position.0 as f64,
                position.1 as f64,
            ));
        self.shell
            .window
            .set_inner_size(tao::dpi::LogicalSize::new(size.0, size.1));
    }
}

impl OsdWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("OSD overlay");
    }
}

impl AuxWindow for OsdWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.shell.window.request_redraw();
    }
}

// ── History viewer ─────────────────────────────────────────────

pub(crate) struct HistoryState {
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

pub(crate) struct HistoryProgram {
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Program for HistoryProgram {
    type State = HistoryState;
    type Message = ();
    type Theme = iced::Theme;
    type Renderer = iced_tiny_skia::Renderer;
    type Executor = iced_winit::futures::backend::default::Executor;

    fn name() -> &'static str {
        "nerust_notification_history"
    }

    fn theme(&self, _state: &Self::State, _window: iced::window::Id) -> Option<Self::Theme> {
        aux_theme()
    }

    fn settings(&self) -> iced::Settings {
        aux_settings()
    }

    fn window(&self) -> Option<iced::window::Settings> {
        None
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        (
            HistoryState {
                shared: std::sync::Arc::clone(&self.shared),
            },
            Task::none(),
        )
    }

    fn update(&self, _state: &mut Self::State, _message: Self::Message) -> Task<Self::Message> {
        Task::none()
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        _window: iced::window::Id,
    ) -> iced::Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        use iced::widget::{column, scrollable, text};
        let lines = state.shared.lock().map(|s| s.clone()).unwrap_or_default();
        let mut items = column![].spacing(2);
        for line in &lines {
            items = items.push(text(line.clone()).size(13));
        }
        scrollable(items).into()
    }
}

pub(crate) struct HistoryWindowHandle {
    pub(crate) shell: crate::aux::shell::AuxShell<HistoryProgram>,
    shared: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl HistoryWindowHandle {
    pub(crate) fn new(
        event_loop: &EventLoopWindowTarget<crate::event::UserEvent>,
        language: AppLanguage,
    ) -> Option<Self> {
        let title = match language {
            AppLanguage::Japanese => "通知履歴",
            _ => "Notification history",
        };
        let scaffold = WindowScaffold::open(
            event_loop,
            title,
            (420.0, 360.0),
            None,
            None,
            "history window",
        )?;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (instance, _task) = program::Instance::new(HistoryProgram {
            shared: std::sync::Arc::clone(&shared),
        });
        let shell = crate::aux::shell::AuxShell::assemble(
            scaffold,
            instance,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        Some(Self { shell, shared })
    }

    pub(crate) fn sync(&mut self, lines: Vec<String>) {
        if let Ok(mut shared) = self.shared.lock() {
            *shared = lines;
        }
        self.shell.window.request_redraw();
    }
}

impl HistoryWindowHandle {
    pub(crate) fn render(&mut self) {
        self.shell.render("history window");
    }
}

impl AuxWindow for HistoryWindowHandle {
    fn pump_window_event(&mut self, event: tao::event::WindowEvent) {
        self.shell.handle_common_event(&event);
        self.shell.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerust_gui_shell::session::notification::{NotificationKind, NotificationLevel};

    fn settings() -> NotificationSettings {
        NotificationSettings::default()
    }

    #[test]
    fn error_modal_collects_and_skips_osd() {
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Error,
                kind: NotificationKind::SlotFailed {
                    slot: Some(1),
                    reason: nerust_gui_shell::session::commands::SlotOpFailure::Storage,
                },
                count: 1,
                seq: 0,
            }],
            &settings(),
            AppLanguage::English,
            now,
        );
        assert!(manager.rows().is_empty());
        assert_eq!(manager.take_modal().len(), 1);
    }

    #[test]
    fn error_osd_holds_five_seconds() {
        let mut altered = settings();
        altered.error_modal = false;
        let mut manager = OsdManager::new();
        let now = Instant::now();
        manager.push_all(
            vec![UiNotification {
                level: NotificationLevel::Info,
                kind: NotificationKind::Paused,
                count: 1,
                seq: 0,
            }],
            &altered,
            AppLanguage::English,
            now,
        );
        // Info expires after 2s.
        assert!(manager.poll(now + Duration::from_millis(2_001)));
        assert!(manager.rows().is_empty());
    }

    #[test]
    fn strip_rows_share_edges() {
        use NotificationPosition as P;
        // Same edge, different sides: one strip.
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                P::BottomRight,
                true
            ),
            (0, 1)
        );
        // Same corner: two stacked strips.
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Strip,
                P::BottomLeft,
                P::BottomLeft,
                true
            ),
            (0, 2)
        );
        // Overlay mode: no strips.
        assert_eq!(
            strip_rows(
                NotificationAreaMode::Overlay,
                P::BottomLeft,
                P::TopLeft,
                true
            ),
            (0, 0)
        );
    }

    #[test]
    fn strip_pixels_scale_with_client_height() {
        assert_eq!(strip_pixels(1000, 1), 30);
        assert_eq!(strip_pixels(0, 1), 0);
        assert_eq!(strip_pixels(1000, 0), 0);
    }
}
