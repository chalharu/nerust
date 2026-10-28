use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nerust_core_traits::audio::AudioBackendRegistry;
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
};
use nerust_gui_shell::{
    registry::SystemRegistry,
    session::{
        notification::{FailureOrigin, Message, NotificationLevel, UiNotification},
        osd::history_lines,
    },
};

use super::*;
use nerust_frontend_host::application::ApplicationHost;

fn lines() -> Vec<HistoryLine> {
    [
        (NotificationLevel::Warning, "warning ×3", 3),
        (NotificationLevel::Info, "info", 1),
        (NotificationLevel::Error, "error", 1),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (level, text, count))| HistoryLine {
        datetime: format!("2026-01-01 00:00:0{index}"),
        level,
        module: "nerust_gui_shell::session::notification",
        text: text.to_string(),
        count,
    })
    .collect()
}

#[test]
fn english_and_japanese_resolve_every_caption_option_and_column_in_order() {
    let lines = lines();
    for (language, captions, headers, labels, levels, count) in [
        (
            AppLanguage::English,
            [
                "Notification history",
                "Close",
                "Level",
                "Clear History",
                "No notifications",
            ],
            ["Datetime", "Level", "Module", "Message"],
            ["All", "Info", "Warning", "Error"],
            ["Warning", "Info", "Error"],
            "3 items",
        ),
        (
            AppLanguage::Japanese,
            [
                "通知履歴",
                "閉じる",
                "レベル",
                "履歴を消去",
                "通知はありません",
            ],
            ["日時", "レベル", "モジュール", "メッセージ"],
            ["すべて", "情報", "警告", "エラー"],
            ["警告", "情報", "エラー"],
            "3件",
        ),
    ] {
        let model = HistoryModel::build(&lines, language, HistoryFilter::All);
        assert_eq!(model.language, language);
        assert_eq!(model.filter, HistoryFilter::All);
        assert_eq!(
            [
                model.captions.title.as_str(),
                model.captions.close.as_str(),
                model.captions.level.as_str(),
                model.captions.clear.as_str(),
                model.captions.empty.as_str(),
            ],
            captions
        );
        assert_eq!(
            model.captions.headers.each_ref().map(String::as_str),
            headers
        );
        assert_eq!(model.captions.count, count);
        assert_eq!(model.options.len(), 4);
        for ((option, filter), label) in model.options.iter().zip(HistoryFilter::all()).zip(labels)
        {
            assert_eq!(option.filter, filter);
            assert_eq!(option.label, label);
        }
        assert_eq!(model.rows.len(), 3);
        for (((row, line), level), rgb) in model.rows.iter().zip(&lines).zip(levels).zip([
            (170, 110, 0),
            (70, 70, 70),
            (190, 30, 30),
        ]) {
            assert_eq!(row.datetime, line.datetime);
            assert_eq!(row.level, level);
            assert_eq!(row.module, "session::notification");
            assert_eq!(row.message, line.text);
            assert_eq!(row.rgb, rgb);
        }
        assert_eq!(model.clone(), model);
    }
}

#[test]
fn all_thresholds_count_visible_rows_not_merged_occurrences_and_allow_empty() {
    let lines = lines();
    for (filter, messages) in [
        (HistoryFilter::All, vec!["warning ×3", "info", "error"]),
        (HistoryFilter::Info, vec!["warning ×3", "info", "error"]),
        (HistoryFilter::Warning, vec!["warning ×3", "error"]),
        (HistoryFilter::Error, vec!["error"]),
    ] {
        for language in [AppLanguage::English, AppLanguage::Japanese] {
            let model = HistoryModel::build(&lines, language, filter);
            assert_eq!(model.filter, filter);
            assert_eq!(
                model
                    .rows
                    .iter()
                    .map(|row| row.message.as_str())
                    .collect::<Vec<_>>(),
                messages
            );
            assert_eq!(
                model.captions.count,
                history_count(language, messages.len())
            );
            for input in [&[][..], &lines[1..2]] {
                let empty = HistoryModel::build(input, language, HistoryFilter::Error);
                assert!(empty.rows.is_empty());
                assert_eq!(empty.captions.count, history_count(language, 0));
                assert!(!empty.captions.empty.is_empty());
            }
        }
    }
}

#[test]
fn shell_rendered_merge_suffix_is_not_appended_twice() {
    let events = [UiNotification {
        level: NotificationLevel::Info,
        message: Message::paused(),
        count: 3,
        seq: 1,
        at: std::time::UNIX_EPOCH,
    }];
    for (language, message) in [
        (AppLanguage::English, "Paused ×3"),
        (AppLanguage::Japanese, "一時停止しました ×3"),
    ] {
        let lines = history_lines(events.iter(), language);
        let model = HistoryModel::build(&lines, language, HistoryFilter::All);
        assert_eq!(model.rows[0].message, message);
        assert_eq!(model.rows[0].message.matches("×3").count(), 1);
        assert_eq!(model.captions.count, history_count(language, 1));
    }
}

#[derive(Default)]
struct Recorder(Vec<String>);

impl HistoryVisitor for Recorder {
    fn begin(&mut self, model: &HistoryModel) {
        self.0.push(model.captions.title.clone());
    }

    fn row(&mut self, row: &HistoryRowView) {
        self.0.push(row.message.clone());
    }

    fn end(&mut self) {
        self.0.push("end".to_string());
    }
}

#[test]
fn dyn_visitor_begins_then_visits_source_order_then_ends_even_when_empty() {
    for lines in [lines(), vec![]] {
        let model = HistoryModel::build(&lines, AppLanguage::English, HistoryFilter::All);
        let mut recorder = Recorder::default();
        let visitor: &mut dyn HistoryVisitor = &mut recorder;
        model.visit(visitor);
        let mut expected = vec!["Notification history".to_string()];
        expected.extend(lines.iter().map(|line| line.text.clone()));
        expected.push("end".to_string());
        assert_eq!(recorder.0, expected);
    }
}

struct TestRoot(PathBuf);

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn session() -> (TestRoot, SessionHandle) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let root = TestRoot(std::env::temp_dir().join(format!(
        "nerust-host-history-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    )));
    let mut session = SessionHandle::new_with_settings_paths(
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        },
        Arc::new(SystemRegistry::new(vec![])),
        Arc::new(AudioBackendRegistry::new()),
        SettingsPaths::from_root(root.0.clone()),
    )
    .unwrap();
    set_language(&mut session, AppLanguage::English);
    (root, session)
}

fn set_language(session: &mut SessionHandle, language: AppLanguage) {
    let mut settings = session.settings_snapshot().clone();
    settings.shared.general.language = language;
    session.apply_settings(settings).unwrap();
}

#[test]
fn controller_gates_unchanged_inputs_but_delivers_filter_language_clear_and_invalidation() {
    let (_root, mut session) = session();
    let mut controller = HistoryController::new();
    assert_eq!(controller, HistoryController::default());
    assert!(
        controller
            .update(&mut session, HistoryFilter::All)
            .unwrap()
            .rows
            .is_empty()
    );
    for _ in 0..3 {
        assert!(
            controller
                .update(&mut session, HistoryFilter::All)
                .is_none()
        );
    }
    let seq = session.notification_history_seq();
    let info = controller
        .update(&mut session, HistoryFilter::Info)
        .unwrap();
    assert_eq!(session.notification_history_seq(), seq);
    assert_eq!(info.filter, HistoryFilter::Info);
    assert!(
        controller
            .update(&mut session, HistoryFilter::Info)
            .is_none()
    );
    session.report_external_error(FailureOrigin::Host, "boom".into());
    let model = controller
        .update(&mut session, HistoryFilter::Info)
        .unwrap();
    assert_eq!(model.rows.len(), 1);
    assert!(model.rows[0].message.contains("boom"));
    assert!(
        controller
            .update(&mut session, HistoryFilter::Info)
            .is_none()
    );
    let seq = session.notification_history_seq();
    set_language(&mut session, AppLanguage::Japanese);
    assert_eq!(session.notification_history_seq(), seq);
    let japanese = controller
        .update(&mut session, HistoryFilter::Info)
        .unwrap();
    assert_eq!(japanese.language, AppLanguage::Japanese);
    assert_eq!(japanese.captions.title, "通知履歴");
    assert_eq!(japanese.rows[0].level, "エラー");
    assert_ne!(japanese.rows[0].message, model.rows[0].message);
    assert!(
        controller
            .update(&mut session, HistoryFilter::Info)
            .is_none()
    );
    session.clear_notification_history();
    let cleared = controller
        .update(&mut session, HistoryFilter::Info)
        .unwrap();
    assert!(cleared.rows.is_empty());
    assert_eq!(cleared.captions.count, "0件");
    assert!(
        controller
            .update(&mut session, HistoryFilter::Info)
            .is_none()
    );
    controller.invalidate();
    assert_eq!(
        controller.update(&mut session, HistoryFilter::Info),
        Some(cleared)
    );
    assert!(
        controller
            .update(&mut session, HistoryFilter::Info)
            .is_none()
    );
}

#[test]
fn visitor_can_reborrow_external_host_and_controller_without_changing_snapshot() {
    struct Reenter<'a> {
        shared: &'a RefCell<(ApplicationHost, HistoryController)>,
        calls: Vec<&'static str>,
    }
    impl HistoryVisitor for Reenter<'_> {
        fn begin(&mut self, model: &HistoryModel) {
            let mut borrowed = self.shared.borrow_mut();
            let (host, controller) = &mut *borrowed;
            assert!(
                controller
                    .update(host.session_mut(), model.filter)
                    .is_none()
            );
            host.clear_history();
            self.calls.push("begin");
        }

        fn row(&mut self, row: &HistoryRowView) {
            let mut borrowed = self.shared.borrow_mut();
            let (host, controller) = &mut *borrowed;
            assert!(
                controller
                    .update(host.session_mut(), HistoryFilter::All)
                    .unwrap()
                    .rows
                    .is_empty()
            );
            assert!(row.message.contains("before clear"));
            self.calls.push("row");
        }

        fn end(&mut self) {
            let mut borrowed = self.shared.borrow_mut();
            let (host, controller) = &mut *borrowed;
            controller.invalidate();
            assert!(
                controller
                    .update(host.session_mut(), HistoryFilter::All)
                    .is_some()
            );
            self.calls.push("end");
        }
    }
    let (_root, mut session) = session();
    session.report_external_error(FailureOrigin::Host, "before clear".into());
    let shared = RefCell::new((ApplicationHost::new(session), HistoryController::new()));
    let model = {
        let mut borrowed = shared.borrow_mut();
        let (host, controller) = &mut *borrowed;
        controller
            .update(host.session_mut(), HistoryFilter::All)
            .unwrap()
    };
    let before = model.clone();
    let mut visitor = Reenter {
        shared: &shared,
        calls: vec![],
    };
    model.visit(&mut visitor as &mut dyn HistoryVisitor);
    assert_eq!(visitor.calls, ["begin", "row", "end"]);
    assert_eq!(model, before);
}
