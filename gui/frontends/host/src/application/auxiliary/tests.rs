use super::*;
use crate::application::tests::Fixture;
use nerust_frontend_kernel::pause::PauseOwner;

fn reserve(host: &mut ApplicationHost, windows: &mut AuxWindows<u64, ()>) -> AuxId {
    let Some(OpenRequest::Create(id)) = host.reserve_window(windows, AuxKind::Settings).unwrap()
    else {
        panic!("expected fresh settings reservation");
    };
    id
}

#[test]
fn shared_success_applies_before_release_and_preserves_modal_owner() {
    let mut fixture = Fixture::new();
    fixture.load();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let id = reserve(host, &mut windows);
    assert!(host.tick().paused);
    let controller = host
        .prepare_settings_window(
            &mut windows,
            id,
            Arc::new(SystemRegistry::new(Vec::new())),
            Arc::new(AudioBackendRegistry::new()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(windows.window(id).unwrap().title, controller.window().title);
    assert!(windows.complete_open(id, Some(1), ()).is_ok());
    host.set_pause_owner(PauseOwner::Modal, true);
    let snapshot = host.settings_snapshot().clone();
    let (handle, result) = host.close_window(&mut windows, id, Some(snapshot)).unwrap();
    assert_eq!(handle, Some(()));
    assert!(result.unwrap().is_some());
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(host.owns_pause(PauseOwner::Modal));
    assert!(host.tick().paused);
}

#[test]
fn pending_present_failure_and_stale_callbacks_do_not_touch_new_owner() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let old = reserve(host, &mut windows);
    assert_eq!(
        host.reserve_window(&mut windows, AuxKind::Settings)
            .unwrap(),
        Some(OpenRequest::Pending(old))
    );
    assert!(host.cancel_window(&mut windows, old));
    assert!(!host.pause_held());
    let current = reserve(host, &mut windows);
    assert_ne!(old, current);
    assert!(!host.cancel_window(&mut windows, old));
    assert!(
        host.close_window(&mut windows, old, Some(host.settings_snapshot().clone()))
            .is_none()
    );
    assert!(host.owns_pause(PauseOwner::Settings));
    assert!(windows.complete_open(current, Some(2), ()).is_ok());
    assert_eq!(
        host.reserve_window(&mut windows, AuxKind::Settings)
            .unwrap(),
        Some(OpenRequest::Present(current))
    );
    assert!(!host.cancel_window(&mut windows, current));
    assert!(host.owns_pause(PauseOwner::Settings));
    assert!(
        host.close_window(&mut windows, current, None)
            .unwrap()
            .1
            .unwrap()
            .is_none()
    );
    assert!(!host.pause_held());
}

#[test]
fn duplicate_factory_registration_is_rejected_and_reservation_can_roll_back() {
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let id = reserve(host, &mut windows);
    // A real registry cannot contain duplicates. Do not fabricate an invalid
    // registry with unsafe layout tricks merely to exercise defensive checking.
    let registry = std::panic::catch_unwind(|| {
        SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
            nerust_nes_factory::NesFactory::boxed().into(),
        ])
    });
    assert!(registry.is_err());
    assert!(host.cancel_window(&mut windows, id));
    assert!(!windows.contains(AuxKind::Settings));
    assert!(!host.owns_pause(PauseOwner::Settings));
}

#[test]
fn invalid_apply_still_releases_settings_pause_and_removes_current_route() {
    use nerust_settings_core::model::shared::StoragePolicy;
    let mut fixture = Fixture::new();
    let host = &mut fixture.core;
    let mut windows = AuxWindows::new();
    let id = reserve(host, &mut windows);
    assert!(windows.complete_open(id, Some(1), ()).is_ok());
    let mut snapshot = host.settings_snapshot().clone();
    snapshot.shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
    snapshot.shared.persistence.storage_directory = None;
    let (_, result) = host.close_window(&mut windows, id, Some(snapshot)).unwrap();
    assert!(result.is_err());
    assert!(!host.owns_pause(PauseOwner::Settings));
    assert!(!windows.contains(AuxKind::Settings));
}
