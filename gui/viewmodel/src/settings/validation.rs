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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub scope: ValidationScope,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationScope {
    Persistence,
    Audio,
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
