/// Overall validation state for the settings view model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationState {
    pub issues: Vec<ValidationIssue>,
}

/// Shared validation banner policy: the headline is the first issue,
/// storage shows the first persistence-scope issue, submit is allowed
/// only when clean. Both frontends render this identically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationBanner {
    pub can_submit: bool,
    pub headline: Option<String>,
    pub storage: Option<String>,
}

impl ValidationState {
    pub fn can_submit(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn banner(&self) -> ValidationBanner {
        ValidationBanner {
            can_submit: self.can_submit(),
            headline: self.issues.first().map(|issue| issue.message.clone()),
            storage: self
                .for_scope(&ValidationScope::Persistence)
                .next()
                .map(|issue| issue.message.clone()),
        }
    }

    pub fn for_scope(&self, scope: &ValidationScope) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(move |issue| issue.scope == *scope)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub scope: ValidationScope,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationScope {
    Persistence,
    Audio,
    Notifications,
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
                message: "error".into(),
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
                    message: "persistence error".into(),
                },
                ValidationIssue {
                    scope: ValidationScope::Audio,
                    message: "audio error".into(),
                },
            ],
        };
        let persistence: Vec<_> = state.for_scope(&ValidationScope::Persistence).collect();
        assert_eq!(persistence.len(), 1);
        assert_eq!(persistence[0].message, "persistence error");
    }
}

#[cfg(test)]
mod banner_tests {
    use super::*;

    #[test]
    fn banner_headlines_first_issue_and_flags_storage() {
        let state = ValidationState {
            issues: vec![
                ValidationIssue {
                    scope: ValidationScope::Audio,
                    message: "audio".into(),
                },
                ValidationIssue {
                    scope: ValidationScope::Persistence,
                    message: "storage".into(),
                },
            ],
        };
        let banner = state.banner();
        assert!(!banner.can_submit);
        assert_eq!(banner.headline.as_deref(), Some("audio"));
        assert_eq!(banner.storage.as_deref(), Some("storage"));
    }

    #[test]
    fn banner_clean_when_no_issues() {
        let banner = ValidationState { issues: vec![] }.banner();
        assert!(banner.can_submit);
        assert!(banner.headline.is_none());
        assert!(banner.storage.is_none());
    }
}
