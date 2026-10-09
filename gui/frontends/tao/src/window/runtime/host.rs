use std::{path::Path, rc::Rc, sync::Arc, time::Instant};

use nerust_gui_runtime::{
    settings::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
        SettingsSnapshot,
    },
    shell::NativeShellState,
};
use nerust_gui_settings::{app_state::RememberedWindowSize, input::ShortcutAction};
use nerust_gui_shell::{
    context::FrontendContext,
    session::{
        KeyboardShortcut, SessionError, SessionHandle,
        access::{FrontendSession, SettingsResult},
        commands::SessionCommand,
        lifecycle::WindowSize,
    },
    settings::scaling_factor,
};
use nerust_render_traits::{
    SurfaceSize,
    renderer::{GpuFactory, RenderResult},
};
use nerust_settings_core::i18n::{UiText, text};
use rfd::FileDialog;
use tao::{
    dpi::{LogicalSize as TaoLogicalSize, PhysicalSize as TaoPhysicalSize},
    event::{ElementState, KeyEvent},
    event_loop::{ControlFlow, EventLoopWindowTarget},
    window::{Fullscreen, Window as TaoWindow, WindowBuilder, WindowId},
};

use crate::app_menu::{MenuCommand, UserEvent, imp::AppMenu};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum HostAction {
    None,
    RomLoaded,
    Exit,
}

const DEFAULT_FIT_WINDOW_WIDTH: f64 = 960.0;
const DEFAULT_FIT_WINDOW_HEIGHT: f64 = 720.0;

pub(crate) struct HostState {
    window: Option<Arc<TaoWindow>>,
    session: SessionHandle,
    ctx: FrontendContext,
    app_menu: AppMenu,
    shell: NativeShellState,
    pub(crate) settings_window: Option<crate::settings_window::SettingsWindowHandle>,
    // SPIKE (iteration 6, DO NOT MERGE): debugger probe window.
    pub(crate) spike_debug_window: Option<crate::spike_debug_window::SpikeDebugWindowHandle>,
    pub(crate) spike_ppu_window: Option<crate::spike_ppu_window::SpikePpuWindowHandle>,
    // SPIKE (iteration 8, DO NOT MERGE): F4 opener request flag.
    spike_open_requested: bool,
    settings_open: bool,
    resume_after_settings: bool,
    pending_fullscreen_sync: Option<bool>,
    pub(crate) active: bool,
    auto_paused: bool,
}

impl HostState {
    pub(crate) fn new(ctx: FrontendContext, app_menu: AppMenu) -> Self {
        let capabilities = HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        };
        let session = SessionHandle::new(
            capabilities,
            ctx.registry.clone(),
            ctx.audio_registry.clone(),
        )
        .unwrap_or_else(|e| {
            log::error!("failed to create core: {e}");
            std::process::exit(1);
        });
        Self {
            window: None,
            session,
            ctx,
            app_menu,
            shell: NativeShellState::new(),
            settings_window: None,
            spike_debug_window: None,
            spike_ppu_window: None,
            spike_open_requested: false,
            settings_open: false,
            resume_after_settings: false,
            pending_fullscreen_sync: None,
            active: true,
            auto_paused: false,
        }
    }

    pub(crate) fn gpu_factory(&self) -> &Rc<dyn GpuFactory> {
        &self.ctx.gpu_factory
    }

    pub(crate) fn session(&self) -> &SessionHandle {
        &self.session
    }

    pub(crate) fn session_mut(&mut self) -> &mut SessionHandle {
        &mut self.session
    }

    pub(crate) fn window(&self) -> Option<&Arc<TaoWindow>> {
        self.window.as_ref()
    }

    pub(crate) fn resume_session(&mut self) {
        FrontendSession::resume(self);
    }

    pub(crate) fn pause_session(&mut self) {
        FrontendSession::pause(self);
    }

    pub(crate) fn auto_paused(&self) -> bool {
        self.auto_paused
    }

    pub(crate) fn set_auto_paused(&mut self) {
        self.auto_paused = true;
    }

    pub(crate) fn clear_auto_paused(&mut self) {
        self.auto_paused = false;
    }

    pub(crate) fn ensure_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.window.is_some() {
            return;
        }

        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        let window = Arc::new(
            create_window_builder(
                self.startup_window_size(),
                self.session.window_title(),
                fullscreen,
            )
            .build(event_loop)
            .unwrap(),
        );
        self.app_menu.init_for_window(&window);
        self.window = Some(window);
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    pub(crate) fn is_window(&self, window_id: WindowId) -> bool {
        self.window
            .as_ref()
            .is_some_and(|window| window.id() == window_id)
    }

    pub(crate) fn is_settings_window(&self, window_id: WindowId) -> bool {
        self.settings_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    // SPIKE (iteration 6, DO NOT MERGE).
    pub(crate) fn is_spike_debug_window(&self, window_id: WindowId) -> bool {
        self.spike_debug_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    // SPIKE (iteration 10, DO NOT MERGE): PPU separate-window routing.
    pub(crate) fn is_spike_ppu_window(&self, window_id: WindowId) -> bool {
        self.spike_ppu_window
            .as_ref()
            .is_some_and(|h| h.window.id() == window_id)
    }

    // SPIKE (iteration 8, DO NOT MERGE): consume the F4 opener flag.
    pub(crate) fn take_spike_open_request(&mut self) -> bool {
        std::mem::replace(&mut self.spike_open_requested, false)
    }

    pub(crate) fn window_surface_size(&self) -> Option<SurfaceSize> {
        self.window
            .as_ref()
            .map(|window| window_surface_size(window.inner_size()))
    }

    pub(crate) fn load_path(&mut self, path: &Path) -> bool {
        let result = self.ctx.rom_loader.load_rom(path, &mut self.session);
        match result {
            Ok(()) => {
                self.after_rom_load();
                true
            }
            Err(e) => {
                log::warn!("ROM load failed: {e}");
                false
            }
        }
    }

    pub(crate) fn on_menu_command(
        &mut self,
        command: MenuCommand,
        event_loop: &EventLoopWindowTarget<UserEvent>,
    ) -> HostAction {
        if self.settings_open {
            return match command {
                MenuCommand::Quit => {
                    if self.prepare_close() {
                        HostAction::Exit
                    } else {
                        HostAction::None
                    }
                }
                _ => HostAction::None,
            };
        }
        match command {
            MenuCommand::Open => {
                if self.open_rom_dialog() {
                    HostAction::RomLoaded
                } else {
                    HostAction::None
                }
            }
            MenuCommand::Settings => {
                self.open_settings_window(event_loop);
                HostAction::None
            }
            // SPIKE (iteration 6, DO NOT MERGE).
            MenuCommand::SpikeDebug => {
                self.open_spike_debug_window(event_loop);
                self.sync_menu_state();
                self.refresh_window_title();
                HostAction::None
            }
            MenuCommand::Session(command) => {
                self.run_command(command);
                self.sync_menu_state();
                self.refresh_window_title();
                HostAction::None
            }
            MenuCommand::Quit => {
                if self.prepare_close() {
                    HostAction::Exit
                } else {
                    HostAction::None
                }
            }
        }
    }

    pub(crate) fn on_keyboard_input(&mut self, input: KeyEvent) {
        if self.settings_open {
            return;
        }
        // SPIKE (iteration 8, DO NOT MERGE): hardcoded F4 opens the
        // debugger probe. A settings-schema `ShortcutAction` addition
        // is production surface and out of spike scope; F4 is free in
        // the default bindings (F5-F8 slots, F11 fullscreen, Space
        // pause). The runtime consumes `take_spike_open_request`.
        if input.state == ElementState::Pressed && input.physical_key == tao::keyboard::KeyCode::F4
        {
            self.spike_open_requested = true;
            return;
        }
        if let Some(pressed) = element_state_to_pressed(input.state)
            && let Some(key) = input.physical_key.try_into().ok()
            && let Some(shortcut) = self.session.handle_keyboard_key(key, pressed)
        {
            self.apply_keyboard_shortcut(shortcut);
        }
    }

    pub(crate) fn clear_keys(&mut self) {
        self.session.clear_input();
    }

    pub(crate) fn sync_fullscreen_default_from_window(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = window_is_fullscreen(window);
        let sync = derive_native_fullscreen_sync(
            self.pending_fullscreen_sync,
            fullscreen,
            self.session
                .settings_snapshot()
                .local
                .video
                .window
                .fullscreen_default,
        );
        self.pending_fullscreen_sync = sync.pending_target;
        if !sync.persist_setting {
            return;
        }
        match self.session.set_fullscreen_default(fullscreen) {
            Ok(plan) if plan.fullscreen_default_changed => {
                self.sync_menu_state();
                self.refresh_window_title();
                self.request_redraw();
            }
            Ok(_) => (),
            Err(error) => log::warn!("failed to persist fullscreen setting: {error}"),
        }
    }

    pub(crate) fn update_control_flow(&mut self, control_flow: &mut ControlFlow) {
        self.sync_fullscreen_default_from_window();
        self.maybe_refresh_window_title(Instant::now());
        *control_flow = ControlFlow::Wait;

        // On macOS, request_redraw() integrates with CVDisplayLink/vsync.
        // On other platforms, it fires on the next event loop iteration.
        // When inactive, no redraw is requested — event loop sleeps (CPU 0%).
        if self.active
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }

    pub(crate) fn on_render_result(&mut self, result: RenderResult) {
        match result {
            RenderResult::Presented => {
                self.shell
                    .on_frame_presented(self.session.metrics().frame_counter);
                self.maybe_refresh_window_title(Instant::now());
            }
            RenderResult::Skipped | RenderResult::Error => {}
        }
    }

    pub(crate) fn clear_event_handler(&self) {
        self.app_menu.clear_event_handler();
    }

    pub(crate) fn request_redraw(&mut self) {
        self.shell.needs_redraw = true;
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    pub(crate) fn prepare_close(&mut self) -> bool {
        self.remember_fit_window_size();
        self.settings_open = false;
        self.resume_after_settings = false;
        self.settings_window.take();
        self.session.flush_before_exit();
        true
    }

    pub(crate) fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let result = FrontendSession::apply_settings(self, settings)?;
        if result.fullscreen_default_changed {
            self.sync_fullscreen_from_settings();
        }
        if result.scaling_changed {
            self.update_window_size_for_scaling();
        }
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(result)
    }

    pub(crate) fn on_settings_closed(&mut self) {
        self.settings_window = None;
        if !self.settings_open {
            return;
        }
        self.settings_open = false;
        let should_resume = std::mem::take(&mut self.resume_after_settings);
        if should_resume {
            self.resume();
        } else {
            self.sync_menu_state();
            self.refresh_window_title();
        }
    }

    fn open_rom_dialog(&mut self) -> bool {
        let mut dialog = FileDialog::new().set_title(text(
            self.session.settings_snapshot().shared.general.language,
            UiText::Open,
        ));
        for factory in self.ctx.registry.all() {
            let extensions = factory.supported_extensions();
            if !extensions.is_empty() {
                dialog = dialog.add_filter(format!("{} ROM", factory.display_name()), extensions);
            }
        }
        dialog.pick_file().is_some_and(|path| self.load_path(&path))
    }

    fn after_rom_load(&mut self) {
        self.sync_menu_state();
        self.request_redraw();
        self.refresh_window_title();
    }

    fn sync_menu_state(&mut self) {
        self.app_menu.update(
            self.session.loaded(),
            self.session.paused(),
            self.session.slots(),
            self.session.active_slot_id(),
            self.settings_open,
            self.session.settings_snapshot().shared.general.language,
        );
    }

    fn refresh_window_title(&mut self) {
        if let Some(window) = self.window.as_ref() {
            window.set_title(&self.session.window_title());
        }
    }

    fn maybe_refresh_window_title(&mut self, now: Instant) {
        if self.shell.should_refresh_title(now) {
            self.refresh_window_title();
        }
    }

    fn apply_keyboard_shortcut(&mut self, shortcut: KeyboardShortcut) {
        match shortcut {
            KeyboardShortcut::Session(action) => match action {
                ShortcutAction::TogglePause => self.toggle_pause(),
                ShortcutAction::SaveActiveSlot => {
                    let _ = self.save_active_slot();
                }
                ShortcutAction::SelectNextSlot => self.select_next_slot(),
                ShortcutAction::SelectPreviousSlot => self.select_previous_slot(),
                ShortcutAction::LoadActiveSlot => {
                    let _ = self.load_active_slot();
                }
                ShortcutAction::Reset => self.reset(),
                ShortcutAction::ToggleFullscreen => self.toggle_fullscreen(),
            },
            KeyboardShortcut::ToggleFullscreen => self.toggle_fullscreen(),
        }
        self.sync_menu_state();
        self.refresh_window_title();
    }

    fn toggle_fullscreen(&mut self) {
        let fullscreen = if let Some(window) = self.window.as_ref() {
            !window_is_fullscreen(window)
        } else {
            return;
        };
        if let Err(error) = self.persist_fullscreen_default(fullscreen) {
            log::warn!("failed to persist fullscreen setting: {error}");
            if let Some(window) = self.window.as_ref() {
                set_window_fullscreen(window, fullscreen);
            }
            self.request_redraw();
        }
    }

    fn open_settings_window(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if self.settings_open {
            return;
        }
        self.settings_open = true;
        self.resume_after_settings = self.session.loaded() && !self.session.paused();
        if self.resume_after_settings {
            self.pause();
        } else {
            self.sync_menu_state();
        }

        match crate::settings_window::SettingsWindowHandle::new(
            self.session.settings_snapshot().clone(),
            self.ctx.registry.clone(),
            self.ctx.audio_registry.clone(),
            event_loop,
        ) {
            Some(handle) => self.settings_window = Some(handle),
            None => {
                log::error!("failed to open settings window");
                self.settings_open = false;
                if self.resume_after_settings {
                    self.resume();
                } else {
                    self.sync_menu_state();
                }
            }
        }
    }

    // SPIKE (iteration 6, DO NOT MERGE): debugger probe lifecycle.
    // Pauses first (all three sections are pause-gated), then opens with
    // the first space selected. Space buttons re-read on selection.
    // SPIKE (iteration 10): also opens the PPU separate window sharing
    // the same bridge; F4 re-opens whichever side was closed.
    pub(crate) fn open_spike_debug_window(
        &mut self,
        event_loop: &EventLoopWindowTarget<UserEvent>,
    ) {
        if self.spike_debug_window.is_none() {
            let _ = self.session.spike_pause();
            let spaces: Vec<(nerust_core_traits::debugger::SpaceId, String, u32)> = self
                .session
                .spike_memory_spaces()
                .into_iter()
                .map(|s| (s.id, s.name.to_string(), *s.range.start()))
                .collect();
            if spaces.is_empty() {
                log::warn!("spike debugger: no spaces (no core)");
            }
            let mem_addr = spaces.first().map(|(_, _, start)| *start).unwrap_or(0);
            // SPIKE (iteration 11): initial fill from the same batch
            // the drain uses (single inspect + disassemble + images).
            let snapshot = if spaces.is_empty() {
                nerust_gui_shell::session::SpikeSnapshot {
                    regs: self.session.spike_registers_text(),
                    panels: self.session.spike_panels_text(),
                    ..Default::default()
                }
            } else {
                self.session
                    .spike_snapshot(Some(spaces[0].0), mem_addr, None)
            };
            let (regs, dump_rows, disasm_lines, panels, images) = (
                snapshot.regs,
                snapshot.dump_rows,
                snapshot.disasm_lines,
                snapshot.panels,
                snapshot.images,
            );
            match crate::spike_debug_window::SpikeDebugWindowHandle::new(
                regs,
                dump_rows,
                disasm_lines,
                panels,
                images,
                spaces,
                mem_addr,
                event_loop,
            ) {
                Some(handle) => self.spike_debug_window = Some(handle),
                None => log::error!("failed to open spike debugger window"),
            }
        }
        if self.spike_ppu_window.is_none()
            && let Some(main) = self.spike_debug_window.as_ref()
        {
            match crate::spike_ppu_window::SpikePpuWindowHandle::new(
                Arc::clone(&main.bridge),
                event_loop,
            ) {
                Some(handle) => self.spike_ppu_window = Some(handle),
                None => log::error!("failed to open spike PPU window"),
            }
        }
    }

    // SPIKE (iteration 6, DO NOT MERGE): auto-open when env-gated and a
    // ROM is already loaded (Xvfb capture needs no driving).
    pub(crate) fn auto_open_spike_debug(&mut self, event_loop: &EventLoopWindowTarget<UserEvent>) {
        if std::env::var("NERUST_SPIKE_DEBUG").is_err() || !self.session.loaded() {
            return;
        }
        self.open_spike_debug_window(event_loop);
    }

    /// SPIKE: drain refresh requests and push fresh section texts back.
    pub(crate) fn drain_spike_debug_requests(&mut self) {
        use crate::spike_debug_window::SpikeDebugRequest;
        use std::sync::atomic::Ordering;
        let requests = match self.spike_debug_window.as_ref() {
            Some(handle) => handle.take_requests(),
            None => return,
        };
        if requests.is_empty() {
            return;
        }
        // Execution requests run before the re-read so one drain both
        // acts and refreshes. The status line reports the last outcome.
        // SPIKE (iteration 11): EditStage/EditCancel only touch the
        // shell transaction; they produce no status here.
        let mut status = String::new();
        for request in &requests {
            let outcome = match request {
                SpikeDebugRequest::Refresh
                | SpikeDebugRequest::MemNav
                | SpikeDebugRequest::EditSelect(_)
                | SpikeDebugRequest::EditStage(_)
                | SpikeDebugRequest::EditCancel
                | SpikeDebugRequest::WriteConfirm => continue,
                SpikeDebugRequest::Pause => self.session.spike_pause(),
                SpikeDebugRequest::Resume => self.session.spike_resume(),
                SpikeDebugRequest::TogglePause => {
                    if self.session.paused() {
                        self.session.spike_resume()
                    } else {
                        self.session.spike_pause()
                    }
                }
                SpikeDebugRequest::StepFrame => self.session.spike_step_frame(),
                SpikeDebugRequest::StepInstr => self.session.spike_step_instruction(),
            };
            status = outcome;
        }
        // SPIKE (iteration 11): cancel/commit hit the shell-owned
        // transaction in request order, before the stale-kill below.
        for request in &requests {
            match request {
                SpikeDebugRequest::EditCancel => self.session.spike_clear_write(),
                SpikeDebugRequest::WriteConfirm => {
                    status = self.session.spike_commit_write();
                    // Iteration-10 parity: a commit consumes the input
                    // so stale digits never pollute the next value.
                    if let Some(handle) = self.spike_debug_window.as_ref() {
                        handle.bridge.edit.lock().unwrap().input.clear();
                    }
                }
                _ => {}
            }
        }
        // SPIKE (iteration 11): state-changing traffic kills
        // staged-but-uncommitted values from previous drains
        // (iteration-9 rule, enforced where the transaction lives).
        // Select/stage/commit traffic never triggers the kill: it
        // builds the transaction this same drain consumes or keeps.
        if requests.iter().any(|request| {
            matches!(
                request,
                SpikeDebugRequest::Refresh
                    | SpikeDebugRequest::MemNav
                    | SpikeDebugRequest::Pause
                    | SpikeDebugRequest::Resume
                    | SpikeDebugRequest::TogglePause
                    | SpikeDebugRequest::StepFrame
                    | SpikeDebugRequest::StepInstr
            )
        }) {
            self.session.spike_clear_write();
        }
        // Row selection prepares the shell transaction (old byte
        // for the confirm row) and mirrors selection display-side.
        for request in &requests {
            if let SpikeDebugRequest::EditSelect(addr) = request {
                let handle = match self.spike_debug_window.as_ref() {
                    Some(handle) => handle,
                    None => return,
                };
                let space = handle.bridge.selected_space();
                let old = space.and_then(|space| self.session.spike_prepare_write(space, *addr));
                let old_text = match old {
                    Some(old) => format!("{old:02X}"),
                    None => "??".to_string(),
                };
                handle.bridge.set_edit_selected(
                    *addr,
                    old,
                    format!("selected {addr:08X} (was {old_text})"),
                );
            }
        }
        // SPIKE (iteration 11): fresh staging after the kill above.
        for request in &requests {
            if let SpikeDebugRequest::EditStage(value) = request {
                self.session.spike_stage_write(*value);
            }
        }
        // SPIKE (iteration 10): frozen values re-apply on every drain
        // (FCEUX freeze style). Silent by design; the rows show it.
        if let Some(handle) = self.spike_debug_window.as_ref()
            && let Some((addr, value)) = *handle.bridge.freeze.lock().unwrap()
            && let Some(space) = handle.bridge.selected_space()
        {
            let _ = self
                .session
                .spike_write_memory(space, addr, 1, u64::from(value));
        }
        let (space, mem_addr, dis_addr) = match self.spike_debug_window.as_ref() {
            Some(handle) => {
                let mem_addr = *handle.bridge.nav.mem_addr.lock().unwrap();
                let dis_addr = if handle.bridge.nav.follow_pc.load(Ordering::Acquire) {
                    None
                } else {
                    Some(*handle.bridge.nav.dis_addr.lock().unwrap())
                };
                (handle.bridge.selected_space(), mem_addr, dis_addr)
            }
            None => return,
        };
        if status.is_empty() {
            status = if self.session.paused() {
                "paused".to_string()
            } else {
                "running".to_string()
            };
        }
        // SPIKE (iteration 11): one batched snapshot per drain (3
        // thread crossings: inspect, disassemble, images). Confirm
        // text follows the cleared-or-committed transaction above.
        let snapshot = match space {
            Some(space) => self.session.spike_snapshot(Some(space), mem_addr, dis_addr),
            None => self.session.spike_snapshot(None, mem_addr, dis_addr),
        };
        let pending_text = self.session.spike_pending_text();
        // SPIKE (iteration 10): watch value fills here (read-only live
        // value, BGB style); changed rows diff against the previous
        // re-read for `*` marks (Mesen style, row-level).
        let (watch_value, diff) = match self.spike_debug_window.as_ref() {
            Some(handle) => {
                let watch_value = handle
                    .bridge
                    .watch
                    .lock()
                    .unwrap()
                    .and_then(|addr| space.and_then(|s| self.session.spike_read_byte(s, addr)));
                let prev = handle.bridge.dump_rows.lock().unwrap().clone();
                let diff: Vec<u32> = snapshot
                    .dump_rows
                    .iter()
                    .filter(|(addr, text)| {
                        prev.iter()
                            .any(|(prev_addr, prev_text)| prev_addr == addr && prev_text != text)
                    })
                    .map(|(addr, _)| *addr)
                    .collect();
                (watch_value, diff)
            }
            None => (None, Vec::new()),
        };
        if let Some(handle) = self.spike_debug_window.as_mut() {
            *handle.bridge.pending_text.lock().unwrap() = pending_text;
            handle.set_all(
                snapshot.regs,
                snapshot.dump_rows,
                snapshot.disasm_lines,
                snapshot.panels,
                snapshot.images,
                status,
                watch_value,
                diff,
            );
        }
    }

    pub(crate) fn close_settings_window(
        &mut self,
        mut handle: crate::settings_window::SettingsWindowHandle,
    ) -> Option<SettingsResult> {
        let pending = handle.take_pending_apply();
        drop(handle);
        self.on_settings_closed();
        if let Some(snapshot) = pending {
            match self.apply_settings(snapshot) {
                Ok(plan) => return Some(plan),
                Err(error) => {
                    log::warn!("settings apply failed after close: {error}");
                }
            }
        }
        None
    }

    fn startup_window_size(&self) -> TaoLogicalSize<f64> {
        match scaling_factor(self.session.settings_snapshot().local.video.window.scaling) {
            Some(scale) => logical_window_size(self.session.window_size(), Some(scale)),
            None => self
                .remembered_fit_window_size()
                .map(logical_size_from_remembered)
                .unwrap_or_else(default_fit_window_size),
        }
    }

    fn update_window_size_for_scaling(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(scale) =
            scaling_factor(self.session.settings_snapshot().local.video.window.scaling)
        else {
            return;
        };
        window.set_inner_size(logical_window_size(self.session.window_size(), Some(scale)));
    }

    fn sync_fullscreen_from_settings(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let fullscreen = self
            .session
            .settings_snapshot()
            .local
            .video
            .window
            .fullscreen_default;
        if window_is_fullscreen(window) == fullscreen {
            self.pending_fullscreen_sync = None;
            return;
        }
        self.pending_fullscreen_sync = Some(fullscreen);
        set_window_fullscreen(window, fullscreen);
    }

    fn persist_fullscreen_default(
        &mut self,
        fullscreen: bool,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        self.sync_fullscreen_from_settings();
        self.sync_menu_state();
        self.refresh_window_title();
        self.request_redraw();
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.session_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }

    fn remembered_fit_window_size(&self) -> Option<RememberedWindowSize> {
        self.session
            .settings_snapshot()
            .app_state
            .window_size("main")
    }

    fn remember_fit_window_size(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window_is_fullscreen(window)
            || scaling_factor(self.session.settings_snapshot().local.video.window.scaling).is_some()
        {
            return;
        }

        let logical_size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let width = logical_size.width.round().max(1.0) as u32;
        let height = logical_size.height.round().max(1.0) as u32;

        if let Err(error) = self
            .session
            .settings_manager()
            .update_window_size(width, height)
        {
            log::warn!("failed to remember tao window size: {error}");
        }
    }
}

impl FrontendSession for HostState {
    fn pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::Pause);
    }
    fn resume(&mut self) {
        let _ = self.session.run_command(SessionCommand::Resume);
    }
    fn toggle_pause(&mut self) {
        let _ = self.session.run_command(SessionCommand::TogglePause);
    }
    fn save_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::SaveActiveSlotOrNew)
            .unwrap_or_default()
            .executed
    }
    fn load_active_slot(&mut self) -> bool {
        self.session
            .run_command(SessionCommand::LoadActiveSlot)
            .unwrap_or_default()
            .executed
    }
    fn select_next_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectNextSlot);
    }
    fn select_previous_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::SelectPreviousSlot);
    }
    fn load_slot(&mut self, slot_id: u64) -> bool {
        self.session
            .run_command(SessionCommand::LoadSlot(slot_id))
            .unwrap_or_default()
            .executed
    }
    fn save_slot(&mut self, slot_id: u64) {
        let _ = self.session.run_command(SessionCommand::SaveSlot(slot_id));
    }
    fn delete_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::DeleteSlot(slot_id));
    }
    fn select_slot(&mut self, slot_id: u64) {
        let _ = self
            .session
            .run_command(SessionCommand::SelectActiveSlot(slot_id));
    }
    fn create_slot(&mut self) {
        let _ = self.session.run_command(SessionCommand::CreateSlot);
    }
    fn reset(&mut self) {
        let _ = self.session.run_command(SessionCommand::Reset);
    }
    fn run_command(&mut self, command: SessionCommand) {
        let _ = self.session.run_command(command);
    }
    fn apply_settings(
        &mut self,
        settings: SettingsSnapshot,
    ) -> Result<SettingsResult, SessionError> {
        let plan = self.session.apply_settings(settings)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.session_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: plan.scaling_changed,
        })
    }
    fn set_fullscreen_default(&mut self, fullscreen: bool) -> Result<SettingsResult, SessionError> {
        let plan = self.session.set_fullscreen_default(fullscreen)?;
        Ok(SettingsResult {
            renderer_needs_rebuild: plan.session_rebuild_required || plan.window_settings_changed,
            fullscreen_default_changed: plan.fullscreen_default_changed,
            scaling_changed: false,
        })
    }
}

fn create_window_builder(
    size: TaoLogicalSize<f64>,
    title: String,
    fullscreen: bool,
) -> WindowBuilder {
    WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_fullscreen(fullscreen_mode(fullscreen))
}

fn window_is_fullscreen(window: &TaoWindow) -> bool {
    window.fullscreen().is_some()
}

fn set_window_fullscreen(window: &TaoWindow, fullscreen: bool) {
    window.set_fullscreen(fullscreen_mode(fullscreen));
}

fn fullscreen_mode(fullscreen: bool) -> Option<Fullscreen> {
    fullscreen.then_some(Fullscreen::Borderless(None))
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct NativeFullscreenSync {
    pending_target: Option<bool>,
    persist_setting: bool,
}

fn derive_native_fullscreen_sync(
    pending_target: Option<bool>,
    fullscreen: bool,
    persisted_fullscreen: bool,
) -> NativeFullscreenSync {
    if let Some(expected) = pending_target
        && fullscreen != expected
    {
        return NativeFullscreenSync {
            pending_target: Some(expected),
            persist_setting: false,
        };
    }

    NativeFullscreenSync {
        pending_target: None,
        persist_setting: fullscreen != persisted_fullscreen,
    }
}

fn window_surface_size(size: TaoPhysicalSize<u32>) -> SurfaceSize {
    SurfaceSize::new(size.width, size.height)
}

fn logical_window_size(size: WindowSize, scaling: Option<u32>) -> TaoLogicalSize<f64> {
    let (width, height) = scaling
        .map(|scale| {
            (
                f64::from(size.width) * f64::from(scale),
                f64::from(size.height) * f64::from(scale),
            )
        })
        .unwrap_or((f64::from(size.width), f64::from(size.height)));
    TaoLogicalSize::new(width, height)
}

fn default_fit_window_size() -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(DEFAULT_FIT_WINDOW_WIDTH, DEFAULT_FIT_WINDOW_HEIGHT)
}

fn logical_size_from_remembered(size: RememberedWindowSize) -> TaoLogicalSize<f64> {
    TaoLogicalSize::new(f64::from(size.width), f64::from(size.height))
}

fn element_state_to_pressed(state: ElementState) -> Option<bool> {
    Some(match state {
        ElementState::Pressed => true,
        ElementState::Released => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use tao::{dpi::LogicalSize as TaoLogicalSize, window::Fullscreen};

    use super::{NativeFullscreenSync, create_window_builder, derive_native_fullscreen_sync};

    #[test]
    fn window_builder_requests_initial_fullscreen_when_enabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), true);

        assert_eq!(
            builder.window.fullscreen,
            Some(Fullscreen::Borderless(None))
        );
    }

    #[test]
    fn window_builder_skips_initial_fullscreen_when_disabled() {
        let builder =
            create_window_builder(TaoLogicalSize::new(960.0, 720.0), "nerust".into(), false);

        assert_eq!(builder.window.fullscreen, None);
    }

    #[test]
    fn fullscreen_sync_ignores_transient_state_while_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), false, true),
            NativeFullscreenSync {
                pending_target: Some(true),
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_clears_pending_transition_once_window_matches_target() {
        assert_eq!(
            derive_native_fullscreen_sync(Some(true), true, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: false,
            }
        );
    }

    #[test]
    fn fullscreen_sync_persists_native_changes_once_no_transition_is_pending() {
        assert_eq!(
            derive_native_fullscreen_sync(None, false, true),
            NativeFullscreenSync {
                pending_target: None,
                persist_setting: true,
            }
        );
    }
}
