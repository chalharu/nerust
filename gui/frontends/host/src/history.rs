//! Owned notification-history presentation, without native widgets or lifetimes.
//! Build under the session borrow, release it, then visit or cache the model.

use nerust_gui_shell::session::{
    SessionHandle,
    osd::{HistoryFilter, HistoryLine, HistoryRowView, history_row_view, visible_history},
};
use nerust_settings_core::{
    i18n::{UiText, history_count, text as ui_text},
    model::language::AppLanguage,
};

/// Localized chrome, including headers in datetime/level/module/message order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCaptions {
    pub title: String,
    pub close: String,
    pub level: String,
    pub clear: String,
    pub count: String,
    pub empty: String,
    pub headers: [String; 4],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryFilterOption {
    pub filter: HistoryFilter,
    pub label: String,
}

/// Render-ready snapshot. Frontends may clone strings into retained controls or iced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryModel {
    pub language: AppLanguage,
    pub filter: HistoryFilter,
    pub captions: HistoryCaptions,
    pub options: Vec<HistoryFilterOption>,
    pub rows: Vec<HistoryRowView>,
}

impl HistoryModel {
    pub fn build(lines: &[HistoryLine], language: AppLanguage, filter: HistoryFilter) -> Self {
        let rows: Vec<_> = visible_history(lines, filter)
            .into_iter()
            .map(|line| history_row_view(line, language))
            .collect();
        Self {
            language,
            filter,
            captions: HistoryCaptions {
                title: ui_text(language, UiText::NotificationHistory).to_string(),
                close: ui_text(language, UiText::Close).to_string(),
                level: ui_text(language, UiText::HistoryLevel).to_string(),
                clear: ui_text(language, UiText::HistoryClear).to_string(),
                count: history_count(language, rows.len()),
                empty: ui_text(language, UiText::HistoryEmpty).to_string(),
                headers: [
                    UiText::HistoryDatetime,
                    UiText::HistoryLevel,
                    UiText::HistoryModule,
                    UiText::HistoryMessage,
                ]
                .map(|key| ui_text(language, key).to_string()),
            },
            options: HistoryFilter::all()
                .into_iter()
                .map(|filter| HistoryFilterOption {
                    filter,
                    label: filter.label(language).to_string(),
                })
                .collect(),
            rows,
        }
    }

    /// Calls begin, each visible row in source order, then end (also when empty).
    /// This borrows only the owned snapshot, never the controller or session.
    pub fn visit(&self, visitor: &mut dyn HistoryVisitor) {
        visitor.begin(self);
        for row in &self.rows {
            visitor.row(row);
        }
        visitor.end();
    }
}

/// Object-safe presentation callbacks; native code may synchronously reenter its host.
pub trait HistoryVisitor {
    fn begin(&mut self, model: &HistoryModel);
    fn row(&mut self, row: &HistoryRowView);
    fn end(&mut self);
}

/// Input-identity gate only: no retained model, native state or presentation borrow.
/// Invalidate when reopening or switching sessions, or if an update was not delivered.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HistoryController {
    key: Option<(u64, AppLanguage, HistoryFilter)>,
}

impl HistoryController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Unchanged inputs return before retrieving or formatting any history lines.
    pub fn update(
        &mut self,
        session: &mut SessionHandle,
        filter: HistoryFilter,
    ) -> Option<HistoryModel> {
        let language = session.settings_snapshot().shared.general.language;
        let key = (session.notification_history_seq(), language, filter);
        if self.key == Some(key) {
            return None;
        }
        let model = HistoryModel::build(
            session.notification_history_lines_cached(language),
            language,
            filter,
        );
        self.key = Some(key);
        Some(model)
    }

    pub fn invalidate(&mut self) {
        self.key = None;
    }
}

#[cfg(test)]
mod tests;
