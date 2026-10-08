//! SPIKE-ONLY data-path test. Deleted with the spike branch.
//!
//! Drives a real NES core through the spike accessors: load, pause,
//! spaces, inspect, step, inspect. Proves the session layer already
//! publishes everything the debug UI needs, with no kernel changes.

use std::sync::Arc;

use nerust_core_traits::{
    audio::AudioBackendRegistry,
    debugger::{InspectRequest, StepUnit},
    factory::{CoreFactory, load::MediaObject},
};
use nerust_gui_runtime::settings::{
    HostBackendCapabilities, HostWindowCapabilities,
};

use super::SessionHandle;
use super::test_util::test_view;
use crate::{registry::SystemRegistry, session::RomLoadTarget, test_helpers::test_rom};

fn nes_session() -> SessionHandle {
    let registry = Arc::new(SystemRegistry::new(vec![Arc::new(
        nerust_nes_factory::NesFactory,
    )]));
    let audio_registry = Arc::new(AudioBackendRegistry::new());
    let capabilities = HostBackendCapabilities {
        window: HostWindowCapabilities {
            remembers_window_size: false,
            supports_fullscreen_default: false,
            supports_scaling: false,
        },
        presentation: None,
    };
    let mut session =
        SessionHandle::new_ephemeral(capabilities, registry, audio_registry);
    session
        .set_active_system(
            nerust_nes_factory::NesFactory
                .system_id()
                .as_ref(),
        )
        .expect("spike: NES activation must work");
    let options = session
        .factory()
        .expect("spike: NES factory must resolve")
        .default_load_options();
    let resolved = session
        .factory()
        .expect("spike: NES factory must resolve")
        .resolve_load_request(&test_view(&session), options)
        .expect("spike: synthetic NROM must resolve");
    session
        .load_resolved(MediaObject::new(None, test_rom()), resolved)
        .expect("spike: synthetic NROM must load");
    assert!(session.loaded());
    session
}

#[test]
fn spike_nes_inspect_returns_views() {
    let session = nes_session();
    session.spike_pause().expect("spike: pause must work");

    let spaces = session.spike_spaces().expect("spike: spaces must work");
    assert!(!spaces.is_empty(), "spike: NES must expose spaces");

    let res = session
        .spike_inspect(InspectRequest {
            space: Some(spaces[0].id),
            addr: Some(0),
            rows: 16,
        })
        .expect("spike: inspect transport must work")
        .expect("spike: paused inspect must succeed");
    assert_eq!(res.dump.rows.len(), 16);
    assert!(!res.registers.is_empty());
}

#[test]
fn spike_nes_step_advances_frame() {
    let session = nes_session();
    session.spike_pause().expect("spike: pause must work");
    let spaces = session.spike_spaces().expect("spike: spaces must work");
    let req = InspectRequest {
        space: Some(spaces[0].id),
        addr: Some(0),
        rows: 1,
    };
    let before = session
        .spike_inspect(req)
        .expect("spike: inspect must work")
        .expect("spike: paused inspect must succeed")
        .captured_at_frame;
    session
        .spike_step(StepUnit::Frame)
        .expect("spike: step transport must work")
        .expect("spike: frame step must succeed");
    let after = session
        .spike_inspect(req)
        .expect("spike: inspect must work")
        .expect("spike: paused inspect must succeed")
        .captured_at_frame;
    assert!(after > before, "spike: frame step must advance");
}
