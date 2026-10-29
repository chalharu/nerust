use std::{
    cell::Cell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::{HexRow, SpaceId};
use nerust_gui_viewmodel::debugger::{DisplayCache, StatusMessage};
use nerust_settings_core::model::language::AppLanguage;

use super::*;
use crate::{application::ApplicationHost, debugger::DebugBridge};

struct Probe<T> {
    ready: Rc<Cell<bool>>,
    polls: Rc<Cell<usize>>,
    output: Option<T>,
}

impl<T> Probe<T> {
    fn new(output: T, ready: bool) -> Self {
        Self {
            ready: Rc::new(Cell::new(ready)),
            polls: Rc::new(Cell::new(0)),
            output: Some(output),
        }
    }
}

impl<T> Flight for Probe<T> {
    type Output = T;
    fn poll(&mut self) -> Poll<T> {
        self.polls.set(self.polls.get() + 1);
        if self.ready.get() {
            Poll::Ready(self.output.take().unwrap())
        } else {
            Poll::Pending
        }
    }
}

type Driver = DebuggerDriver<Probe<Box<DebugDrainOutput>>, Probe<LiveImages>>;

fn bridge() -> DebugBridge {
    DebugBridge::new(
        DisplayCache::default(),
        vec![(SpaceId(0), "Memory".into(), 0)],
        0,
        AppLanguage::English,
    )
}

fn test_epoch() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

fn output() -> Box<DebugDrainOutput> {
    Box::new(DebugDrainOutput {
        display: DisplayCache {
            paused: true,
            ..Default::default()
        },
        reselected: Some((0x12, Some(0xab))),
    })
}

#[test]
fn in_flight_fifo_retains_actions_and_gesture_snapshot_across_pause_boundaries() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    bridge.test_state(|core| {
        core.freeze = Some((0x12, 0x42));
        core.reg_target = Some("a");
        core.nav.go_mem(0x100);
    });
    driver.enqueue_current(
        &epoch,
        &bridge,
        &[
            DebugAction::SelectRow(0x12),
            DebugAction::EditStage(0x42),
            DebugAction::Pause,
            DebugAction::RegisterWrite(7),
            DebugAction::Resume,
        ],
        DebuggerScope::Dump,
        true,
    );
    let flight = Probe::new(output(), false);
    let ready = flight.ready.clone();
    let mut flight = Some(flight);
    let first = driver.poll_with(&bridge, |actions, snapshot| {
        assert!(matches!(
            actions,
            [DebugAction::SelectRow(0x12), DebugAction::EditStage(0x42)]
        ));
        assert_eq!(snapshot.mem_addr, 0x100);
        assert_eq!(snapshot.freeze, Some((0x12, 0x42)));
        assert!(snapshot.ppu_open);
        flight.take().unwrap()
    });
    assert!(first.pending);
    assert_eq!(first.scope, None);
    bridge.test_state(|core| {
        core.nav.go_mem(0x200);
        core.freeze = None;
        core.reg_target = None;
    });
    driver.enqueue_current(
        &epoch,
        &bridge,
        &[DebugAction::WriteConfirm],
        DebuggerScope::State,
        false,
    );
    assert!(
        driver
            .poll_with(&bridge, |_, _| panic!("must wait for first flight"))
            .pending
    );
    ready.set(true);
    let mut started = 0;
    let effects = driver.poll_with(&bridge, |actions, snapshot| {
        match started {
            0 => {
                assert!(matches!(
                    actions,
                    [DebugAction::Pause, DebugAction::RegisterWrite(7)]
                ));
                assert_eq!(snapshot.reg_target, Some("a"));
                assert_eq!(snapshot.freeze, Some((0x12, 0x42)));
            }
            1 => assert!(matches!(actions, [DebugAction::Resume])),
            2 => {
                assert!(matches!(actions, [DebugAction::WriteConfirm]));
                assert_eq!(snapshot.mem_addr, 0x200);
                assert!(!snapshot.ppu_open);
                assert_eq!(snapshot.freeze, None);
            }
            _ => panic!("unexpected batch"),
        }
        started += 1;
        Probe::new(output(), true)
    });
    assert_eq!(started, 3);
    assert!(!effects.pending);
    assert_eq!(effects.scope, Some(DebuggerScope::Dump));
    assert_eq!(bridge.selected(), Some((0x12, Some(0xab))));
}

#[test]
fn completed_scopes_union_even_when_the_next_flight_is_pending() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    driver.enqueue_current(&epoch, &bridge, &[], DebuggerScope::Disasm, false);
    driver.enqueue_current(&epoch, &bridge, &[], DebuggerScope::Dump, false);
    driver.enqueue_current(&epoch, &bridge, &[], DebuggerScope::State, false);
    let mut starts = 0;
    let effects = driver.poll_with(&bridge, |_, _| {
        starts += 1;
        Probe::new(output(), starts < 3)
    });
    assert!(effects.pending);
    assert_eq!(effects.scope, Some(DebuggerScope::All));
    assert_eq!(bridge.selected(), Some((0x12, Some(0xab))));
    assert_eq!(
        union(Some(DebuggerScope::Dump), DebuggerScope::Edit),
        DebuggerScope::Dump
    );
    assert_eq!(
        union(Some(DebuggerScope::Header), DebuggerScope::Bookmarks),
        DebuggerScope::Bookmarks
    );
}

#[test]
fn retired_epoch_drops_pending_output_and_queued_pause_without_polling() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    driver.enqueue_current(
        &epoch,
        &bridge,
        &[DebugAction::StepInstr, DebugAction::Resume],
        DebuggerScope::All,
        false,
    );
    let flight = Probe::new(output(), false);
    let polls = flight.polls.clone();
    let mut flight = Some(flight);
    assert!(
        driver
            .poll_with(&bridge, |_, _| flight.take().unwrap())
            .pending
    );
    epoch.store(false, Ordering::Release);
    let effects = driver.poll_with(&bridge, |_, _| panic!("old pause cannot begin"));
    assert!(!effects.pending);
    assert_eq!(effects.scope, None);
    assert!(bridge.selected().is_none());
    assert_eq!(polls.get(), 1);
}

fn live(generation: u64) -> LiveImages {
    // The shell exposes LiveImages but intentionally hides its concrete batch
    // type. Seed through that public API without expanding shell exports.
    static SEED: std::sync::OnceLock<LiveImages> = std::sync::OnceLock::new();
    let mut images = SEED
        .get_or_init(|| {
            let mut fixture = Fixture::new();
            fixture.load();
            let mut flight = fixture.host.session().debug_live_images_begin(true);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            loop {
                match Flight::poll(&mut flight) {
                    Poll::Ready(images) => {
                        assert!(images.is_some());
                        break images;
                    }
                    Poll::Pending => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::yield_now();
                    }
                }
            }
        })
        .clone();
    let batch = images.as_mut().unwrap();
    batch.generation = generation;
    batch.images = vec![("Tiles".into(), 1, 1, vec![1u8, 2, 3, 255]).into()];
    images
}

#[test]
fn live_interest_single_flight_and_paused_late_reply_preserve_inspection() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    bridge.update_display(|display| {
        display.paused = true;
        display.status = StatusMessage::Paused;
        display.dump_rows = vec![HexRow {
            addr: 0,
            bytes: [0xab; 16],
            valid: 16,
        }];
    });
    for (running, interested) in [(false, true), (true, false), (false, false)] {
        let effects = driver.live_tick_with(&bridge, epoch.clone(), running, interested, || {
            panic!("not interested")
        });
        assert!(!effects.pending);
        assert!(!effects.changed);
    }
    let flight = Probe::new(live(4), false);
    let ready = flight.ready.clone();
    assert!(
        driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || flight)
            .pending
    );
    assert!(
        driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || panic!(
                "single flight"
            ))
            .pending
    );
    ready.set(true);
    let effects =
        driver.live_tick_with(&bridge, epoch.clone(), false, true, || panic!("now paused"));
    assert!(effects.changed);
    assert!(!effects.pending);
    assert!(bridge.display_snapshot().paused);
    assert_eq!(bridge.display_snapshot().status, StatusMessage::Paused);
    assert_eq!(
        bridge.display_snapshot().images[0].rgba,
        vec![1u8, 2, 3, 255]
    );
    assert_eq!(bridge.display_snapshot().dump_rows[0].bytes, [0xab; 16]);
    assert!(
        !driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || Probe::new(
                live(4),
                true
            ))
            .changed
    );
    assert!(
        bridge.display_snapshot().paused,
        "duplicate generation must not change chrome"
    );
    assert!(
        driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || Probe::new(
                live(5),
                true
            ))
            .changed
    );
    assert_eq!(bridge.display_snapshot().status, StatusMessage::Running);
    assert_eq!(bridge.display_snapshot().dump_rows[0].bytes, [0xab; 16]);
}

#[test]
fn retirement_and_replacement_drop_live_and_all_mutation_state() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    let flight = Probe::new(live(9), false);
    let polls = flight.polls.clone();
    driver.begin_live(epoch.clone(), || flight);
    epoch.store(false, Ordering::Release);
    assert!(!driver.poll_live_with(&bridge, true).changed);
    assert_eq!(polls.get(), 0);
    assert!(!driver.live_pending());
    bridge.test_state(|core| {
        core.watch = Some(1);
        core.freeze = Some((1, 0xff));
        core.reg_target = Some("a");
        core.selected = Some((1, None));
        core.bookmarks.push((1, "old".into()));
    });
    let live_epoch = test_epoch();
    driver.enqueue_current(
        &live_epoch,
        &bridge,
        &[DebugAction::WriteConfirm],
        DebuggerScope::All,
        false,
    );
    assert!(driver.pending());
    // A fresh session epoch retires flights and queued batches; the app
    // rebuilds the bridge, so the queued mutation never reaches a display.
    let fixture = Fixture::new();
    assert!(driver.reset_replaced_epoch(fixture.host.session()));
    assert!(!driver.pending());
    assert!(!driver.live_pending());
    assert!(!driver.reset_replaced_epoch(fixture.host.session()));
    assert_eq!(bridge.display_snapshot().images_gen, 0);
}

#[test]
fn imageless_live_completion_releases_the_single_flight_slot() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    bridge.update_display(|display| {
        display.status = StatusMessage::Paused;
    });
    let flight = Probe::new(None, false);
    let ready = flight.ready.clone();
    assert!(
        driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || flight)
            .pending
    );
    ready.set(true);
    let effects = driver.live_tick_with(&bridge, epoch.clone(), false, false, || {
        panic!("no interest")
    });
    assert!(!effects.pending);
    assert!(!effects.changed);
    assert!(!driver.live_pending());
    assert_eq!(bridge.display_snapshot().status, StatusMessage::Paused);
    assert!(
        driver
            .live_tick_with(&bridge, epoch.clone(), true, true, || Probe::new(
                live(7),
                true
            ))
            .changed
    );
}

#[test]
fn running_drain_preserves_inspection_and_publishes_running_images() {
    let bridge = bridge();
    let epoch = test_epoch();
    let mut driver = Driver::new();
    bridge.update_display(|display| {
        display.paused = true;
        display.regs = vec![("a", 7)];
        display.dump_rows = vec![HexRow {
            addr: 0x10,
            bytes: [0xab; 16],
            valid: 16,
        }];
    });
    driver.live_tick_with(&bridge, epoch.clone(), true, true, || {
        Probe::new(live(8), true)
    });
    driver.enqueue_current(&epoch, &bridge, &[], DebuggerScope::Header, false);
    let effects = driver.poll_with(&bridge, |_, snapshot| {
        assert_eq!(snapshot.prev_rows[0].addr, 0x10);
        Probe::new(
            Box::new(DebugDrainOutput {
                display: DisplayCache {
                    paused: false,
                    status: StatusMessage::Running,
                    images_gen: 9,
                    images: vec![DebugImageRgba {
                        label: "Running".into(),
                        width: 1,
                        height: 1,
                        rgba: vec![4u8, 5, 6, 255],
                    }],
                    ..Default::default()
                },
                reselected: Some((0x12, Some(0xab))),
            }),
            true,
        )
    });
    assert_eq!(effects.scope, Some(DebuggerScope::Header));
    assert!(!bridge.display_snapshot().paused);
    assert_eq!(bridge.display_snapshot().regs, vec![("a", 7)]);
    assert_eq!(bridge.display_snapshot().dump_rows[0].addr, 0x10);
    assert_eq!(bridge.display_snapshot().images_gen, 9);
    assert_eq!(
        bridge.display_snapshot().images[0].rgba,
        vec![4u8, 5, 6, 255]
    );
    assert_eq!(bridge.selected(), Some((0x12, Some(0xab))));
}

struct Fixture {
    host: ApplicationHost,
    loader: Box<dyn nerust_gui_shell::load::RomLoader>,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nerust-debug-driver-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let registry = Arc::new(nerust_gui_shell::registry::SystemRegistry::new(vec![
            nerust_nes_factory::NesFactory::boxed().into(),
        ]));
        let loader = registry.create_loader(Default::default()).unwrap();
        let session = SessionHandle::new_with_settings_paths(
            nerust_gui_runtime::settings::HostBackendCapabilities {
                window: nerust_gui_runtime::settings::HostWindowCapabilities {
                    remembers_window_size: true,
                    supports_fullscreen_default: true,
                    supports_scaling: true,
                },
                presentation: None,
            },
            registry,
            Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
            nerust_gui_runtime::settings::SettingsPaths::from_root(root.clone()),
        )
        .unwrap();
        Self {
            host: ApplicationHost::new(session),
            loader,
            root,
        }
    }

    fn load(&mut self) {
        let mut rom = vec![0u8; 16 + 16384 + 8192];
        rom[..6].copy_from_slice(&[b'N', b'E', b'S', 0x1a, 1, 1]);
        rom[16..19].copy_from_slice(&[0x4c, 0, 0x80]);
        for vector in [0x3ffa, 0x3ffc, 0x3ffe] {
            rom[16 + vector..18 + vector].copy_from_slice(&[0, 0x80]);
        }
        let path = self.root.join("loop.nes");
        std::fs::write(&path, rom).unwrap();
        self.host.load_path(self.loader.as_mut(), &path).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.begin_close();
        self.host.finish_close();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn replacement_rejects_stale_local_gesture_and_input_echo_then_refreshes() {
    let mut fixture = Fixture::new();
    fixture.load();
    let bridge = fixture.host.new_debug_bridge();
    let mut driver = Driver::new();
    assert!(driver.reset_replaced_epoch(fixture.host.session()));
    bridge.test_state(|core| {
        core.watch = Some(0);
        core.freeze = Some((0, 0xff));
    });
    {
        let epoch = fixture.host.session().debug_epoch();
        driver.enqueue_current(
            &epoch,
            &bridge,
            &[DebugAction::Resume],
            DebuggerScope::All,
            false,
        );
    }
    driver.begin_live(fixture.host.session().debug_epoch(), || {
        Probe::new(live(4), false)
    });
    fixture.load();
    let effects = driver.dispatch(
        fixture.host.session(),
        &bridge,
        [DebuggerEvent::BookmarkInputChanged("stale".into())],
        DebuggerEvent::BookmarkAdd,
        false,
    );
    assert!(effects.epoch_reset);
    assert!(!effects.open_ppu);
    // The app rebuilds the bridge on replacement; stale input never survives.
    let bridge = fixture.host.new_debug_bridge();
    bridge.bind_epoch(fixture.host.session().debug_epoch());
    bridge.test_state(|core| {
        assert!(core.bookmarks.is_empty());
        assert!(core.bm_input.is_empty());
        assert!(core.freeze.is_none());
    });
    assert!(!driver.live_pending());
    let mut starts = 0;
    let effects = driver.poll_with(&bridge, |actions, _| {
        starts += 1;
        assert!(matches!(actions, [DebugAction::Refresh]));
        Probe::new(output(), true)
    });
    assert_eq!(starts, 1);
    assert_eq!(effects.scope, Some(DebuggerScope::All));

    fixture.load();
    driver.enqueue(
        fixture.host.session(),
        &bridge,
        &[DebugAction::Resume, DebugAction::WriteConfirm],
        DebuggerScope::Edit,
        false,
    );
    driver.poll_with(&bridge, |actions, _| {
        assert!(matches!(actions, [DebugAction::Refresh]));
        Probe::new(output(), true)
    });
}

#[test]
fn ppu_open_captures_image_interest_and_typing_never_drains() {
    let fixture = Fixture::new();
    let bridge = fixture.host.new_debug_bridge();
    let mut driver = Driver::new();
    assert!(driver.reset_replaced_epoch(fixture.host.session()));
    let effects = driver.dispatch(
        fixture.host.session(),
        &bridge,
        [],
        DebuggerEvent::EditInputChanged("42".into()),
        false,
    );
    assert_eq!(effects.scope, None);
    assert!(!driver.pending());
    let effects = driver.dispatch(
        fixture.host.session(),
        &bridge,
        [],
        DebuggerEvent::PpuOpenPressed,
        false,
    );
    assert!(effects.open_ppu);
    driver.poll_with(&bridge, |actions, snapshot| {
        assert!(matches!(actions, [DebugAction::Refresh]));
        assert!(snapshot.ppu_open);
        Probe::new(output(), true)
    });
}

fn settle(driver: &mut DebuggerDriver, bridge: &DebugBridge, host: &mut ApplicationHost) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while host.poll_driver_with(driver, bridge) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn actual_host_pause_ownership_composes_with_modal_and_replacement() {
    use crate::session::pause_owners;
    let mut fixture = Fixture::new();
    fixture.load();
    let bridge = fixture.host.new_debug_bridge();
    let mut driver = DebuggerDriver::new();
    assert!(driver.reset_replaced_epoch(fixture.host.session()));
    driver.enqueue(
        fixture.host.session(),
        &bridge,
        &[DebugAction::Pause, DebugAction::Refresh],
        DebuggerScope::All,
        false,
    );
    settle(&mut driver, &bridge, &mut fixture.host);
    assert!(fixture.host.owns_pause(pause_owners::USER));
    fixture.host.set_pause_owner(pause_owners::MODAL, true);
    driver.dispatch(
        fixture.host.session(),
        &bridge,
        [],
        DebuggerEvent::TogglePausePressed,
        false,
    );
    settle(&mut driver, &bridge, &mut fixture.host);
    assert!(!fixture.host.owns_pause(pause_owners::USER));
    assert!(fixture.host.tick().paused, "modal still owns pause");
    driver.enqueue(
        fixture.host.session(),
        &bridge,
        &[DebugAction::Pause],
        DebuggerScope::All,
        false,
    );
    settle(&mut driver, &bridge, &mut fixture.host);
    fixture.load();
    driver.dispatch(
        fixture.host.session(),
        &bridge,
        [],
        DebuggerEvent::TogglePausePressed,
        false,
    );
    settle(&mut driver, &bridge, &mut fixture.host);
    fixture.host.set_pause_owner(pause_owners::MODAL, false);
    assert!(fixture.host.owns_pause(pause_owners::USER));
    assert!(
        fixture.host.tick().paused,
        "stale resume must not release user pause"
    );
}

#[test]
fn coreless_pause_keeps_shell_status_and_render_snapshot_excludes_images() {
    let mut fixture = Fixture::new();
    let bridge = fixture.host.new_debug_bridge();
    let mut driver = DebuggerDriver::new();
    assert!(driver.reset_replaced_epoch(fixture.host.session()));
    driver.enqueue(
        fixture.host.session(),
        &bridge,
        &[DebugAction::Pause],
        DebuggerScope::All,
        false,
    );
    settle(&mut driver, &bridge, &mut fixture.host);
    assert_eq!(bridge.display_snapshot().status, StatusMessage::NoCore);
    bridge.update_display(|display| {
        display.images = vec![DebugImageRgba {
            label: "Tiles".into(),
            width: 1,
            height: 1,
            rgba: vec![0u8; 4],
        }];
    });
    bridge.set_edit_input("42".into());
    let snapshot = bridge.view_snapshot();
    assert!(snapshot.display.images.is_empty());
    assert_eq!(snapshot.edit_input, "42");
    assert_eq!(bridge.display_snapshot().images.len(), 1);
}
