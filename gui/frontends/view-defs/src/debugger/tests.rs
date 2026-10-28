use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use nerust_core_traits::debugger::{DEBUG_DUMP_PAGE_ROWS, DUMP_ROW_BYTES, DisasmLine, SpaceId};
use nerust_gui_shell::debug::DebugAction;
use nerust_gui_viewmodel::debugger::{DebugImageRgba, DisplayCache, PendingWrite, StatusMessage};
use nerust_settings_core::model::language::AppLanguage;

use super::{DebuggerEvent::*, DebuggerScope::*, *};

fn model() -> DebuggerModel {
    DebuggerModel::new(
        DisplayCache::default(),
        vec![(SpaceId(0), "Memory".into(), 0)],
        0,
        AppLanguage::English,
        Arc::new(AtomicBool::new(true)),
    )
}

fn bridge() -> DebugBridge {
    DebugBridge::new(
        DisplayCache::default(),
        vec![(SpaceId(0), "Memory".into(), 0)],
        0,
        AppLanguage::English,
    )
}

fn line(addr: u32, is_pc: bool) -> DisasmLine {
    DisasmLine {
        addr,
        bytes: [0; 4],
        len: 1,
        text: "NOP".into(),
        is_pc,
        target: None,
    }
}

fn status(model: &DebuggerModel) -> &str {
    match &model.display.status {
        StatusMessage::Local(text) => text,
        other => panic!("expected local status, got {other:?}"),
    }
}

// Eight pure-update regressions moved out of the native Tao adapter.
#[test]
fn confirm_write_pushes_write_confirm_and_clears_input() {
    let mut model = model();
    model.dispatch(EditInputChanged("42".into()));
    let update = model.dispatch(ConfirmWrite);
    assert!(matches!(
        update.actions.as_slice(),
        [DebugAction::WriteConfirm]
    ));
    assert_eq!(update.scope, Some(All));
    assert!(model.edit_input.is_empty());
}

#[test]
fn cancel_write_pushes_edit_cancel_and_clears_pending() {
    let mut model = model();
    model.display.pending = Some(PendingWrite {
        addr: 0x100,
        old: Some(0xab),
        value: 0x42,
    });
    let update = model.dispatch(CancelWrite);
    assert!(matches!(
        update.actions.as_slice(),
        [DebugAction::EditCancel]
    ));
    assert_eq!(update.scope, Some(Edit));
    assert!(model.display.pending.is_none());
}

#[test]
fn bookmark_add_defaults_label_and_falls_back_to_pin() {
    let mut model = model();
    model.dispatch(DisLineClicked(0x123));
    let update = model.dispatch(BookmarkAdd);
    assert_eq!(model.bookmarks, vec![(0x123, "BM1".into())]);
    assert!(model.bm_input.is_empty());
    assert!(update.actions.is_empty());
    assert_eq!(update.scope, Some(Bookmarks));
}

#[test]
fn freeze_rejects_unparseable_byte_with_selection() {
    let mut model = model();
    model.selected = Some((0x100, Some(0xab)));
    model.dispatch(FreezeInputChanged("ZZZ".into()));
    let update = model.dispatch(FreezeSet);
    assert!(model.freeze.is_none());
    assert!(update.actions.is_empty());
    assert_eq!(update.scope, Some(Header));
    assert_eq!(status(&model), "freeze needs a selected row and hex byte");
}

#[test]
fn freeze_rejects_out_of_range_byte() {
    let mut model = model();
    model.selected = Some((0x100, None));
    model.dispatch(FreezeInputChanged("1FF".into()));
    assert!(model.dispatch(FreezeSet).actions.is_empty());
    assert!(model.freeze.is_none());
    assert_eq!(status(&model), "freeze needs a selected row and hex byte");
}

#[test]
fn edit_write_rejects_bad_hex_without_queueing() {
    let mut model = model();
    model.selected = Some((0x100, Some(0xab)));
    model.dispatch(EditInputChanged(" ZZZ ".into()));
    let update = model.dispatch(EditWrite);
    assert!(update.actions.is_empty());
    assert_eq!(update.scope, Some(Header));
    assert_eq!(status(&model), "parse failed: ZZZ");
}

#[test]
fn cycle_space_without_spaces_pushes_nothing() {
    let mut model = DebuggerModel::new(
        DisplayCache::default(),
        vec![],
        0,
        AppLanguage::English,
        Arc::new(AtomicBool::new(true)),
    );
    model.selected = Some((0x100, None));
    for event in [SpaceNext, SpacePrev] {
        let update = model.dispatch(event);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, None);
    }
    assert!(model.selected.is_some());
    assert_eq!(model.selected_space(), None);
    assert_eq!(model.selected_name(), "");
}

#[test]
fn reg_set_accepts_full_width_hex() {
    let mut model = model();
    model.dispatch(RegSelected("r0"));
    for (input, expected) in [("1FF", 0x1ff), (" $FFFFFFFF ", u64::from(u32::MAX))] {
        model.dispatch(RegInputChanged(input.into()));
        let update = model.dispatch(RegSet);
        assert!(
            matches!(update.actions.as_slice(), [DebugAction::RegisterWrite(value)] if *value == expected)
        );
        assert_eq!(update.scope, Some(State));
    }
    model.dispatch(RegInputChanged("100000000".into()));
    assert!(model.dispatch(RegSet).actions.is_empty());
}

#[test]
fn bookmarks_are_unique_capped_trimmed_and_local() {
    let mut model = model();
    for i in 0..8 {
        model.dispatch(DisLineClicked(i));
        model.dispatch(BookmarkInputChanged(if i == 0 {
            " tag ".into()
        } else {
            "  ".into()
        }));
        let update = model.dispatch(BookmarkAdd);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, Some(Bookmarks));
    }
    assert_eq!(model.bookmarks[0].1, "tag");
    assert_eq!(model.bookmarks[7].1, "BM8");
    // Even at capacity, a duplicate is a harmless local no-op, not an error.
    model.dispatch(BookmarkInputChanged("replacement".into()));
    assert_eq!(model.dispatch(BookmarkAdd).scope, Some(Bookmarks));
    assert_eq!(model.bookmarks.len(), 8);
    assert_eq!(model.bookmarks[7].1, "BM8");
    assert!(model.bm_input.is_empty());
    model.dispatch(DisLineClicked(99));
    let update = model.dispatch(BookmarkAdd);
    assert_eq!(update.scope, Some(Header));
    assert!(update.actions.is_empty());
    assert_eq!(status(&model), "bookmark list full");
    let update = model.dispatch(BookmarkDelete(0));
    assert!(update.actions.is_empty());
    assert_eq!(update.scope, Some(Bookmarks));
    assert_eq!(model.bookmarks.len(), 7);
}

#[test]
fn bookmark_follow_prefers_pc_but_pin_always_wins() {
    let mut model = model();
    model.display.disasm_lines = vec![line(0x100, false), line(0x104, true)];
    assert_eq!(model.anchor(), 0x100);
    model.dispatch(BookmarkAdd);
    assert_eq!(model.bookmarks[0].0, 0x104);
    model.dispatch(DisLineClicked(0x300));
    model.dispatch(BookmarkAdd);
    assert_eq!(model.bookmarks[1].0, 0x300);
    model.dispatch(FollowPcToggle);
    model.display.disasm_lines[1].is_pc = false;
    model.dispatch(BookmarkAdd);
    assert_eq!(model.bookmarks[2].0, 0x100);
    model.display.disasm_lines.clear();
    model.dispatch(BookmarkAdd);
    assert_eq!(model.bookmarks[3].0, 0);
}

#[test]
fn typing_has_no_traffic_scope_or_image_copies_in_either_storage() {
    let mut model = model();
    model.display.images = vec![DebugImageRgba {
        label: "Tiles".into(),
        width: 1,
        height: 1,
        rgba: vec![0xff; 4],
    }];
    let bridge = DebugBridge::new(
        model.display.clone(),
        model.spaces.clone(),
        0,
        model.language,
    );
    let before = bridge.display_snapshot();
    let owned_bytes = model.display.images[0].rgba.as_ptr();
    for event in [
        MemInputChanged("100".into()),
        DisInputChanged("200".into()),
        BookmarkInputChanged("tag".into()),
        FreezeInputChanged("FF".into()),
        EditInputChanged("42".into()),
        RegInputChanged("7".into()),
    ] {
        let update = model.dispatch(event.clone());
        let shared = bridge.dispatch(event);
        assert!(update.actions.is_empty() && shared.actions.is_empty());
        assert_eq!((update.scope, shared.scope), (None, None));
        assert!(!update.open_ppu && !shared.open_ppu);
        assert!(Arc::ptr_eq(&before, &bridge.display_snapshot()));
        assert_eq!(owned_bytes, model.display.images[0].rgba.as_ptr());
    }
    assert!(bridge.take_requests().is_empty());
    assert!(bridge.view_snapshot().display.images.is_empty());
    // Display-clearing intents with nothing to clear also preserve the Arc.
    bridge.dispatch(WatchClear);
    bridge.dispatch(CancelWrite);
    assert!(Arc::ptr_eq(&before, &bridge.display_snapshot()));
}

#[test]
fn navigation_space_selection_and_scopes() {
    let mut model = DebuggerModel::new(
        DisplayCache::default(),
        vec![
            (SpaceId(0), "Memory".into(), 0),
            (SpaceId(1), "Other".into(), 0x8000),
        ],
        0,
        AppLanguage::English,
        Arc::new(AtomicBool::new(true)),
    );
    model.dispatch(MemInputChanged(" $100 ".into()));
    assert_eq!(model.dispatch(MemGo).scope, Some(Dump));
    assert_eq!(model.nav.mem_addr(), 0x100);
    model.dispatch(MemPage(1));
    assert_eq!(
        model.nav.mem_addr(),
        0x100 + DEBUG_DUMP_PAGE_ROWS as u32 * DUMP_ROW_BYTES as u32
    );
    model.dispatch(MemPage(i32::MIN));
    assert_eq!(model.nav.mem_addr(), 0);
    model.selected = Some((0x10, None));
    assert_eq!(model.dispatch(SpaceNext).scope, Some(Dump));
    assert_eq!(model.selected_space(), Some(SpaceId(1)));
    assert_eq!(model.selected_name(), "Other");
    assert_eq!(model.nav.mem_addr(), 0x8000);
    assert!(model.selected.is_none());
    model.dispatch(SpaceNext);
    assert_eq!(model.selected_space(), Some(SpaceId(0)));
    model.dispatch(SpacePrev);
    assert_eq!(model.selected_space(), Some(SpaceId(1)));
    model.display.disasm_lines = vec![line(0x200, true)];
    model.dispatch(DisInputChanged("300".into()));
    assert_eq!(model.dispatch(DisGo).scope, Some(Disasm));
    model.dispatch(FollowTarget(0x400));
    assert_eq!(model.dispatch(DisBack).scope, Some(Disasm));
    assert_eq!(model.anchor(), 0x300);
    model.dispatch(DisBack);
    assert_eq!(model.anchor(), 0x200);
    assert_eq!(model.dispatch(DisBack).scope, Some(Header));
    assert_eq!(model.dispatch(FollowPcToggle).scope, Some(Disasm));
    assert!(model.nav.follow_pc());
    let update = model.dispatch(RowSelected(0x123));
    assert_eq!(update.scope, Some(Dump));
    assert!(matches!(
        update.actions.as_slice(),
        [DebugAction::SelectRow(0x123)]
    ));
    assert!(
        model.selected.is_none(),
        "selection comes back from the drain"
    );
}

#[test]
fn selected_watch_freeze_and_two_phase_write() {
    let mut model = model();
    model.apply_drain(DisplayCache::default(), Some((0x123, Some(0xab))));
    assert_eq!(model.dispatch(WatchSet).scope, Some(State));
    assert_eq!(model.watch, Some(0x123));
    model.display.watch_value = Some(0xab);
    assert!(model.dispatch(WatchClear).actions.is_empty());
    assert!(model.watch.is_none() && model.display.watch_value.is_none());
    model.dispatch(FreezeInputChanged(" ff ".into()));
    assert_eq!(model.dispatch(FreezeSet).scope, Some(All));
    assert_eq!(model.freeze, Some((0x123, 0xff)));
    model.dispatch(EditInputChanged(" $42 ".into()));
    assert_eq!(model.dispatch(EditFreezeSet).scope, Some(All));
    assert_eq!(model.freeze, Some((0x123, 0x42)));
    let update = model.dispatch(EditWrite);
    assert_eq!(update.scope, Some(Dump));
    assert!(matches!(
        update.actions.as_slice(),
        [DebugAction::SelectRow(0x123), DebugAction::EditStage(0x42)]
    ));
    assert!(model.display.pending.is_none(), "stage is not optimistic");
    let update = model.dispatch(FreezeClear);
    assert_eq!(update.scope, Some(State));
    assert!(
        update.actions.is_empty(),
        "clearing is local, not a memory undo"
    );
    assert!(model.freeze.is_none());
    assert_eq!(status(&model), "freeze cleared");
    for input in ["100", "-1", "", " zz "] {
        model.dispatch(EditInputChanged(input.into()));
        let update = model.dispatch(EditWrite);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, Some(Header));
    }
}

#[test]
fn errors_are_header_only_and_localized() {
    let mut model = model();
    for event in [WatchSet, FreezeSet, EditWrite, RegSet] {
        let update = model.dispatch(event);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, Some(Header));
    }
    for (echo, go) in [
        (MemInputChanged(" zz ".into()), MemGo),
        (DisInputChanged(" zz ".into()), DisGo),
    ] {
        model.dispatch(echo);
        let update = model.dispatch(go);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, Some(Header));
        assert_eq!(status(&model), "parse failed: zz");
    }
    model.language = AppLanguage::Japanese;
    model.dispatch(EditWrite);
    assert_eq!(status(&model), "書込には行選択が必要");
}

#[test]
fn owned_and_split_lock_paths_return_identical_updates_and_state() {
    let mut model = model();
    let bridge = bridge();
    model.selected = Some((0x100, None));
    bridge.set_selected_for_test(model.selected);
    for event in [
        RefreshPressed,
        TogglePausePressed,
        StepFramePressed,
        StepInstrPressed,
        PpuOpenPressed,
        MemInputChanged("100".into()),
        MemGo,
        MemPage(-1),
        SpaceNext,
        DisInputChanged("200".into()),
        DisGo,
        FollowTarget(0x300),
        DisBack,
        FollowPcToggle,
        BookmarkInputChanged("tag".into()),
        BookmarkAdd,
        BookmarkJump(0x400),
        BookmarkDelete(0),
        WatchSet,
        WatchClear,
        FreezeInputChanged("AA".into()),
        FreezeSet,
        EditFreezeSet,
        FreezeClear,
        RowSelected(0x100),
        EditInputChanged("42".into()),
        EditWrite,
        CancelWrite,
        ConfirmWrite,
        RegSelected("r0"),
        RegInputChanged("FFFFFFFF".into()),
        RegSet,
    ] {
        let owned = model.dispatch(event.clone());
        let shared = bridge.dispatch(event);
        assert_eq!(
            format!("{:?}", owned.actions),
            format!("{:?}", shared.actions)
        );
        assert_eq!(
            (owned.scope, owned.open_ppu),
            (shared.scope, shared.open_ppu)
        );
        let view = bridge.view_snapshot();
        assert_eq!(format!("{:?}", model.nav), format!("{:?}", view.nav));
        assert_eq!(model.bookmarks, view.bookmarks);
        assert_eq!(model.selected, view.selected);
        assert_eq!(model.watch, view.watch);
        assert_eq!(model.freeze, view.freeze);
        assert_eq!(model.reg_target, view.reg_target);
        assert_eq!(model.edit_input, view.edit_input);
        assert_eq!(model.display.status, view.display.status);
    }
}

#[test]
fn stale_epoch_rejects_entire_batch_and_all_ui_mutations() {
    let bridge = bridge();
    let epoch = Arc::new(AtomicBool::new(true));
    bridge.bind_epoch(epoch.clone());
    bridge.set_selected_for_test(Some((0x100, None)));
    bridge.dispatch(EditInputChanged("42".into()));
    epoch.store(false, Ordering::Release);
    bridge.view_invalidated.store(false, Ordering::Release);
    let display = bridge.display_snapshot();
    for event in [
        EditWrite,
        ConfirmWrite,
        CancelWrite,
        PpuOpenPressed,
        MemInputChanged("stale".into()),
        BookmarkAdd,
        WatchSet,
    ] {
        let update = bridge.dispatch(event);
        assert!(update.actions.is_empty());
        assert_eq!(update.scope, None);
        assert!(!update.open_ppu);
    }
    assert!(bridge.take_requests().is_empty());
    assert_eq!(bridge.view_snapshot().nav.mem_input(), "0x00000000");
    assert_eq!(bridge.edit_input(), "42");
    assert!(bridge.bookmarks().is_empty());
    assert!(bridge.watch().is_none());
    assert!(!bridge.view_invalidated.load(Ordering::Acquire));
    assert!(!bridge.open_ppu.load(Ordering::Acquire));
    assert!(Arc::ptr_eq(&display, &bridge.display_snapshot()));
    let mut model = model();
    model.epoch = epoch;
    assert!(
        model
            .dispatch(EditInputChanged("stale".into()))
            .actions
            .is_empty()
    );
    assert!(model.edit_input.is_empty());
}

#[test]
fn concurrent_takes_never_split_two_phase_batches() {
    let bridge = bridge();
    bridge.bind_epoch(Arc::new(AtomicBool::new(true)));
    bridge.set_selected_for_test(Some((0x100, None)));
    bridge.dispatch(EditInputChanged("42".into()));
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..1000 {
                bridge.dispatch(EditWrite);
            }
            done.store(true, Ordering::Release);
        });
        let mut count = 0;
        loop {
            // Load before taking: the final take must occur after producer completion.
            let finished = done.load(Ordering::Acquire);
            let actions = bridge.take_requests();
            assert_eq!(actions.len() % 2, 0);
            for pair in actions.as_chunks::<2>().0 {
                assert!(matches!(
                    pair,
                    [DebugAction::SelectRow(0x100), DebugAction::EditStage(0x42)]
                ));
                count += 1;
            }
            if finished {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(count, 1000);
    });
}

#[test]
fn display_patches_are_cow_only_when_the_value_changes() {
    let bridge = bridge();
    bridge.update_display(|display| display.watch_value = Some(0xab));
    let before = bridge.display_snapshot();
    bridge.dispatch(WatchClear);
    let after = bridge.display_snapshot();
    assert!(!Arc::ptr_eq(&before, &after));
    assert_eq!(before.watch_value, Some(0xab));
    assert_eq!(after.watch_value, None);
    bridge.dispatch(WatchClear);
    assert!(Arc::ptr_eq(&after, &bridge.display_snapshot()));
    bridge.dispatch(MemInputChanged("zz".into()));
    bridge.dispatch(MemGo);
    let error = bridge.display_snapshot();
    bridge.dispatch(MemGo);
    assert!(Arc::ptr_eq(&error, &bridge.display_snapshot()));
}

#[test]
fn owned_drain_and_live_images_keep_selection_and_generation_contracts() {
    let mut model = model();
    assert!(!model.apply_live_images(0, vec![], true));
    assert!(model.apply_live_images(1, vec![], true));
    assert_eq!(model.display.status, StatusMessage::Running);
    model.display.status = StatusMessage::Local("held".into());
    assert!(model.apply_live_images(2, vec![], false));
    assert_eq!(status(&model), "held");
    model.apply_drain(DisplayCache::default(), Some((0x100, Some(0xab))));
    model.apply_drain(DisplayCache::default(), None);
    assert_eq!(model.selected, Some((0x100, Some(0xab))));
}

#[test]
fn model_from_session_uses_session_metadata_and_epoch() {
    use nerust_gui_runtime::settings::{
        HostBackendCapabilities, HostWindowCapabilities, SettingsPaths,
    };
    use nerust_gui_shell::{registry::SystemRegistry, session::SessionHandle};

    let root = std::env::temp_dir().join(format!(
        "nerust-debugger-model-session-{}",
        std::process::id()
    ));
    let session = SessionHandle::new_with_settings_paths(
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: None,
        },
        Arc::new(SystemRegistry::new(vec![])),
        Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new()),
        SettingsPaths {
            settings_file: root.join("settings.toml"),
            central_storage_root: root.join("storage"),
        },
    )
    .unwrap();
    let model = DebuggerModel::from_session(&session);
    assert!(Arc::ptr_eq(&model.epoch, &session.debug_epoch()));
    assert_eq!(
        model.language,
        session.settings_snapshot().shared.general.language
    );
    assert!(model.spaces.is_empty());
    assert_eq!(model.selected_space(), None);
    assert_eq!(model.nav.mem_addr(), 0);
    drop(session);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn owned_model_initializes_the_first_space_start_without_shadow_state() {
    let mut model = DebuggerModel::new(
        DisplayCache::default(),
        vec![(SpaceId(7), "ROM".into(), 0x8000)],
        0x8000,
        AppLanguage::Japanese,
        Arc::new(AtomicBool::new(true)),
    );
    assert_eq!(model.selected_space(), Some(SpaceId(7)));
    assert_eq!(model.selected_name(), "ROM");
    assert_eq!(model.nav.mem_addr(), 0x8000);
    model.nav.go_mem(0x9000);
    assert_eq!(model.nav.mem_input(), "0x00009000");
    assert_eq!(model.dispatch(MemGo).scope, Some(Dump));
    assert_eq!(model.nav.mem_addr(), 0x9000);
}
