use nerust_settings_core::model::language::AppLanguage;

/// Overall validation state for the settings view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationState {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationState {
    pub fn can_submit(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn for_scope(&self, scope: &ValidationScope) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(move |issue| issue.scope == *scope)
    }
}

/// One draft problem: where it blocks, plus structured data for the
/// message. The view model never renders text; both frontends format
/// through [`ValidationIssueKind::message`] with their display language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub scope: ValidationScope,
    pub kind: ValidationIssueKind,
}

impl ValidationIssue {
    pub fn message(&self, language: AppLanguage) -> String {
        self.kind.message(language)
    }
}

/// Machine-readable validation failure. Opaque platform details
/// (`Storage`, `Placement`, key/conflict symbols) pass through
/// untranslated; only the template sentence localizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationIssueKind {
    /// Platform storage detail (path validator, URI rules): no template.
    Storage(String),
    VolumeOutOfRange,
    SampleRateOutOfRange,
    LatencyOutOfRange,
    InfoDurationOutOfRange,
    WarningDurationOutOfRange,
    UnknownSlots {
        system: String,
        count: u32,
    },
    UnknownProfiles {
        system: String,
        count: u32,
    },
    NoController {
        system: String,
    },
    /// Input-layer placement rejection: detail is the opaque rule text.
    Placement {
        system: String,
        detail: String,
    },
    /// Duplicate physical key: key label plus the joined control labels.
    KeyConflict {
        key: String,
        labels: String,
    },
    /// Duplicate physical pad button: button label plus the joined
    /// shortcut labels. Same shape as [`ValidationIssueKind::KeyConflict`],
    /// but global (shortcuts are not per-system).
    PadConflict {
        button: String,
        labels: String,
    },
    ExclusiveMember {
        member: String,
        ports: String,
    },
}

impl ValidationIssueKind {
    pub fn message(&self, language: AppLanguage) -> String {
        use nerust_settings_core::i18n::{self as core_i18n, I18n, UiText};
        let i18n = I18n::resolve_os(language);
        let lang = i18n.language();
        match self {
            Self::Storage(detail) => detail.clone(),
            Self::VolumeOutOfRange => i18n.text(UiText::ValidationVolumeRange),
            Self::SampleRateOutOfRange => i18n.text(UiText::ValidationSampleRateRange),
            Self::LatencyOutOfRange => i18n.text(UiText::ValidationLatencyRange),
            Self::InfoDurationOutOfRange => i18n.text(UiText::ValidationInfoDuration),
            Self::WarningDurationOutOfRange => i18n.text(UiText::ValidationWarningDuration),
            Self::UnknownSlots { system, count } => core_i18n::unknown_slots(lang, system, *count),
            Self::UnknownProfiles { system, count } => {
                core_i18n::unknown_profiles(lang, system, *count)
            }
            Self::NoController { system } => {
                format!("{system}: {}", i18n.text(UiText::ValidationNoController))
            }
            Self::Placement { system, detail } => format!("{system}: {detail}"),
            Self::KeyConflict { key, labels } => format!("{key}: {labels}"),
            Self::PadConflict { button, labels } => format!("{button}: {labels}"),
            Self::ExclusiveMember { member, ports } => {
                core_i18n::exclusive_member(lang, member, ports)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationScope {
    Persistence,
    Audio,
    Notifications,
    Controllers,
    Input(Box<dyn nerust_core_traits::identity::SystemId>),
    System(Box<dyn nerust_core_traits::identity::SystemId>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_validation_allows_submit() {
        let state = ValidationState { issues: vec![] };
        assert!(state.can_submit());
    }

    #[test]
    fn issues_block_submit() {
        let state = ValidationState {
            issues: vec![ValidationIssue {
                scope: ValidationScope::Persistence,
                kind: ValidationIssueKind::Storage("error".into()),
            }],
        };
        assert!(!state.can_submit());
    }

    #[test]
    fn for_scope_filters_correctly() {
        let state = ValidationState {
            issues: vec![
                ValidationIssue {
                    scope: ValidationScope::Persistence,
                    kind: ValidationIssueKind::Storage("persistence error".into()),
                },
                ValidationIssue {
                    scope: ValidationScope::Audio,
                    kind: ValidationIssueKind::VolumeOutOfRange,
                },
            ],
        };
        let persistence: Vec<_> = state.for_scope(&ValidationScope::Persistence).collect();
        assert_eq!(persistence.len(), 1);
        assert_eq!(
            persistence[0].kind,
            ValidationIssueKind::Storage("persistence error".into())
        );
    }

    #[test]
    fn messages_localize_templates_and_pass_opaque_details_through() {
        let kind = ValidationIssueKind::VolumeOutOfRange;
        assert_eq!(
            kind.message(AppLanguage::English),
            "Master volume must be between 0 and 100"
        );
        assert_eq!(
            kind.message(AppLanguage::Japanese),
            "主音量は0～100で指定してください"
        );
        let opaque = ValidationIssueKind::Storage("custom detail".into());
        assert_eq!(
            opaque.message(AppLanguage::Japanese),
            "custom detail",
            "platform details have no translation"
        );
        let conflict = ValidationIssueKind::ExclusiveMember {
            member: "pad:0".into(),
            ports: "p1, p2".into(),
        };
        assert!(
            conflict
                .message(AppLanguage::Japanese)
                .contains("複数ポート")
        );
        assert!(
            conflict
                .message(AppLanguage::English)
                .contains("multiple ports")
        );
    }
}
