//! Property mapper: the framework-owned vocabulary of control updates.
//!
//! MAUI's `IPropertyMapper` analog. Backends never diff controls by hand:
//! [`map_updates`] computes the canonical update list, retained backends
//! execute it, immediate-mode backends fold the chrome subset into their
//! builders. Payloads borrow the new snapshot; executors apply them
//! synchronously, so no allocation is forced on either side.

use crate::control::Control;
use crate::leaf::LeafUpdate;

/// Single property change, in canonical application order:
/// enabled, visible, label, value, display, then accessibility.
/// Chrome setters may overwrite accessibility, so accessibility sorts last.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PropertyUpdate {
    SetEnabled(bool),
    SetVisible(bool),
    SetLabel(String),
    SetValue(LeafUpdate),
    SetDisplay(crate::control::ControlDisplay),
    SetAccessibleLabel(String),
    SetHelp(Option<String>),
}

/// Compute the update list from `old` to `new`.
/// `None` yields the full property list (construction);
/// `Some` yields only changed aspects. Value diffs delegate to the leaf.
pub fn map_updates(old: Option<&Control>, new: &Control) -> Vec<PropertyUpdate> {
    let mut updates = Vec::new();
    if old.map(|o| o.metadata.enabled) != Some(new.metadata.enabled) {
        updates.push(PropertyUpdate::SetEnabled(new.metadata.enabled));
    }
    if old.map(|o| o.metadata.visible) != Some(new.metadata.visible) {
        updates.push(PropertyUpdate::SetVisible(new.metadata.visible));
    }
    if old.map(|o| o.label.clone()) != Some(new.label.clone()) {
        updates.push(PropertyUpdate::SetLabel(new.label.clone()));
    }
    let old_leaf: Option<&dyn crate::leaf::Leaf> =
        old.map(|o| o.leaf.as_ref() as &dyn crate::leaf::Leaf);
    for value in new.leaf.updates(old_leaf) {
        updates.push(PropertyUpdate::SetValue(value));
    }
    if old.map(|o| o.display) != Some(new.display) {
        updates.push(PropertyUpdate::SetDisplay(new.display));
    }
    if old.map(|o| o.metadata.accessible_label.clone())
        != Some(new.metadata.accessible_label.clone())
    {
        updates.push(PropertyUpdate::SetAccessibleLabel(
            new.metadata.accessible_label.clone(),
        ));
    }
    if old.map(|o| o.metadata.help.clone()) != Some(new.metadata.help.clone()) {
        updates.push(PropertyUpdate::SetHelp(new.metadata.help.clone()));
    }
    updates
}

/// Platform-specific post-processing hook (MAUI `ModifyMapping` analog).
/// Runs after the default mapping; native quirks that need to suppress or
/// rewrite updates live here instead of in every executor.
pub trait MapperRule {
    fn adjust(&self, updates: &mut Vec<PropertyUpdate>, old: Option<&Control>, new: &Control);
}

/// [`map_updates`] plus platform rules, applied in order.
pub fn map_updates_with(
    old: Option<&Control>,
    new: &Control,
    rules: &[&dyn MapperRule],
) -> Vec<PropertyUpdate> {
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
    use crate::leaf::ToggleLeaf;

    fn control(value: bool) -> Control {
        Control {
            id: ControlId::fresh(),
            label: "Volume".into(),
            metadata: ControlMetadata::new("Volume"),
            display: ControlDisplay::Standard,
            leaf: Box::new(ToggleLeaf { value }),
        }
    }

    #[test]
    fn full_list_covers_every_aspect() {
        let new = control(true);
        let updates = map_updates(None, &new);
        assert!(updates.contains(&PropertyUpdate::SetEnabled(true)));
        assert!(updates.contains(&PropertyUpdate::SetVisible(true)));
        assert!(updates.contains(&PropertyUpdate::SetLabel("Volume".into())));
        assert!(updates.contains(&PropertyUpdate::SetValue(LeafUpdate::FullRefresh)));
        assert!(updates.contains(&PropertyUpdate::SetDisplay(ControlDisplay::Standard)));
        assert!(updates.contains(&PropertyUpdate::SetAccessibleLabel("Volume".into())));
        assert!(updates.contains(&PropertyUpdate::SetHelp(None)));
        assert_eq!(updates.len(), 7);
    }

    #[test]
    fn identical_snapshots_emit_nothing() {
        let old = control(true);
        let mut same = control(true);
        same.id = old.id;
        same.label = old.label.clone();
        assert!(map_updates(Some(&old), &same).is_empty());
    }

    #[test]
    fn only_changed_aspects_are_emitted_in_canonical_order() {
        let old = control(false);
        let mut new = control(true);
        new.id = old.id;
        new.label = "Master volume".into();
        let updates = map_updates(Some(&old), &new);
        assert_eq!(
            updates,
            vec![
                PropertyUpdate::SetLabel("Master volume".into()),
                PropertyUpdate::SetValue(LeafUpdate::ValueChanged),
            ]
        );
    }

    #[test]
    fn rules_run_after_the_default_mapping() {
        struct StripHelp;
        impl MapperRule for StripHelp {
            fn adjust(
                &self,
                updates: &mut Vec<PropertyUpdate>,
                _old: Option<&Control>,
                _new: &Control,
            ) {
                updates.retain(|u| !matches!(u, PropertyUpdate::SetHelp(_)));
            }
        }
        let mut with_help = control(true);
        with_help.metadata.help = Some("tip".into());
        let plain = map_updates(None, &with_help);
        assert!(plain.contains(&PropertyUpdate::SetHelp(Some("tip".into()))));
        let stripped = map_updates_with(None, &with_help, &[&StripHelp]);
        assert!(
            !stripped
                .iter()
                .any(|u| matches!(u, PropertyUpdate::SetHelp(_)))
        );
        assert_eq!(stripped.len(), plain.len() - 1);
    }
}
