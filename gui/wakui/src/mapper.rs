//! Property mapper: the framework-owned vocabulary of control updates.
//!
//! MAUI's `IPropertyMapper` analog. Backends never diff controls by hand:
//! [`map_updates`] computes the canonical update list, retained backends
//! execute it, immediate-mode backends fold the chrome subset into their
//! builders. Payloads borrow the new snapshot; executors apply them
//! synchronously, so no allocation is forced on either side.

use crate::control::{Control, ControlDisplay, ControlKind};

/// Single property change, in canonical application order:
/// enabled, visible, label, value, display, then accessibility.
/// Chrome setters may overwrite accessibility, so accessibility sorts last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyUpdate<'a> {
    SetEnabled(bool),
    SetVisible(bool),
    SetLabel(&'a str),
    SetValue(&'a ControlKind),
    SetDisplay(ControlDisplay),
    SetAccessibleLabel(&'a str),
    SetHelp(Option<&'a str>),
}

/// Compute the update list from `old` to `new`.
/// `None` yields the full property list (construction);
/// `Some` yields only changed aspects.
pub fn map_updates<'a>(old: Option<&Control>, new: &'a Control) -> Vec<PropertyUpdate<'a>> {
    let mut updates = Vec::new();
    if old.map(|o| o.metadata.enabled) != Some(new.metadata.enabled) {
        updates.push(PropertyUpdate::SetEnabled(new.metadata.enabled));
    }
    if old.map(|o| o.metadata.visible) != Some(new.metadata.visible) {
        updates.push(PropertyUpdate::SetVisible(new.metadata.visible));
    }
    if old.map(|o| o.label.as_str()) != Some(new.label.as_str()) {
        updates.push(PropertyUpdate::SetLabel(&new.label));
    }
    if old.map(|o| &o.kind) != Some(&new.kind) {
        updates.push(PropertyUpdate::SetValue(&new.kind));
    }
    if old.map(|o| o.display) != Some(new.display) {
        updates.push(PropertyUpdate::SetDisplay(new.display));
    }
    if old.map(|o| o.metadata.accessible_label.as_str())
        != Some(new.metadata.accessible_label.as_str())
    {
        updates.push(PropertyUpdate::SetAccessibleLabel(
            &new.metadata.accessible_label,
        ));
    }
    if old.map(|o| o.metadata.help.as_deref()) != Some(new.metadata.help.as_deref()) {
        updates.push(PropertyUpdate::SetHelp(new.metadata.help.as_deref()));
    }
    updates
}

/// Platform-specific post-processing hook (MAUI `ModifyMapping` analog).
/// Runs after the default mapping; native quirks that need to suppress or
/// rewrite updates live here instead of in every executor.
pub trait MapperRule {
    fn adjust(&self, updates: &mut Vec<PropertyUpdate<'_>>, old: Option<&Control>, new: &Control);
}

/// [`map_updates`] plus platform rules, applied in order.
pub fn map_updates_with<'a>(
    old: Option<&Control>,
    new: &'a Control,
    rules: &[&dyn MapperRule],
) -> Vec<PropertyUpdate<'a>> {
    let mut updates = map_updates(old, new);
    for rule in rules {
        rule.adjust(&mut updates, old, new);
    }
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{ControlDisplay, ControlId, ControlMetadata};

    fn control(kind: ControlKind) -> Control {
        Control {
            id: ControlId::fresh(),
            label: "Volume".into(),
            metadata: ControlMetadata::new("Volume"),
            display: ControlDisplay::Standard,
            kind,
        }
    }

    #[test]
    fn full_list_covers_every_aspect() {
        let new = control(ControlKind::Toggle { value: true });
        let updates = map_updates(None, &new);
        assert!(updates.contains(&PropertyUpdate::SetEnabled(true)));
        assert!(updates.contains(&PropertyUpdate::SetVisible(true)));
        assert!(updates.contains(&PropertyUpdate::SetLabel("Volume")));
        assert!(updates.contains(&PropertyUpdate::SetValue(&new.kind.clone())));
        assert!(updates.contains(&PropertyUpdate::SetDisplay(ControlDisplay::Standard)));
        assert!(updates.contains(&PropertyUpdate::SetAccessibleLabel("Volume")));
        assert!(updates.contains(&PropertyUpdate::SetHelp(None)));
        assert_eq!(updates.len(), 7);
    }

    #[test]
    fn identical_snapshots_emit_nothing() {
        let control = control(ControlKind::Toggle { value: true });
        assert!(map_updates(Some(&control), &control).is_empty());
    }

    #[test]
    fn only_changed_aspects_are_emitted_in_canonical_order() {
        let old = control(ControlKind::Toggle { value: false });
        let mut new = control(ControlKind::Toggle { value: true });
        new.label = "Master volume".into();
        let updates = map_updates(Some(&old), &new);
        assert_eq!(
            updates,
            vec![
                PropertyUpdate::SetLabel("Master volume"),
                PropertyUpdate::SetValue(&new.kind),
            ]
        );
    }

    #[test]
    fn rules_run_after_the_default_mapping() {
        struct StripHelp;
        impl MapperRule for StripHelp {
            fn adjust(
                &self,
                updates: &mut Vec<PropertyUpdate<'_>>,
                _old: Option<&Control>,
                _new: &Control,
            ) {
                updates.retain(|u| !matches!(u, PropertyUpdate::SetHelp(_)));
            }
        }
        let mut with_help = control(ControlKind::Note {
            text: "note".into(),
        });
        with_help.metadata.help = Some("tip".into());
        let plain = map_updates(None, &with_help);
        assert!(plain.contains(&PropertyUpdate::SetHelp(Some("tip"))));
        let stripped = map_updates_with(None, &with_help, &[&StripHelp]);
        assert!(
            !stripped
                .iter()
                .any(|u| matches!(u, PropertyUpdate::SetHelp(_)))
        );
        assert_eq!(stripped.len(), plain.len() - 1);
    }
}
