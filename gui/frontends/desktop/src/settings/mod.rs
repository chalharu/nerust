mod layout;

use layout::{BindingRowCommands, ResetRowCommands};
use nerust_gui_runtime::settings::{
    SettingsSnapshot,
    apply::{DirectoryValidationError, validate_directory_path_typed},
};
use nerust_gui_viewmodel::settings::{
    ReadOnlyObservableProperty, SettingsViewModel, StoragePathError, StoragePathValidator,
    ValidationIssue, ViewModelError,
    dto::{ChoiceView, choice_position},
};
use nerust_settings_core::{
    editor::CaptureTarget,
    i18n::{I18n, UiText},
    model::{
        app_state::DesktopAppState, input::ensure_default_shortcuts, language::AppLanguage,
        local::HostBackendLocalSettings, shared::DesktopSharedSettings,
    },
};
use nerust_system_traits::factory::CoreFactory;
use panechord::backend::{View, WeakView};
use panechord::{Binding, Choice, Command, Executor, Keyed, Store, Subscription};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::Arc,
};

type Setter<T> = Rc<dyn Fn(T) -> Result<(), ViewModelError>>;
pub(crate) type Apply = Rc<dyn Fn(SettingsSnapshot) -> Result<(), String>>;

/// Screen scopes for the two remaining reset buttons: the App tab's
/// reset-all and the Shortcuts page reset. Every other page owns no
/// reset (per-page resets proved barely needed, so they are gone).
#[derive(Clone)]
pub(super) enum ResetScope {
    All,
    Shortcuts,
}

/// Canonical defaults for a reset: stock shared/local/app state plus each
/// factory's default system settings, with canonical shortcut bindings.
/// Mirrors the Android reset source so both frontends restore the same.
fn default_snapshot(factories: &[Arc<dyn CoreFactory>]) -> SettingsSnapshot {
    let mut shared = DesktopSharedSettings::default();
    for factory in factories {
        if let Some(defaults) = factory
            .as_system_defaults()
            .and_then(|system_defaults| system_defaults.default_system_settings())
        {
            shared.systems.insert(factory.system_id(), defaults);
        }
    }
    ensure_default_shortcuts(&mut shared.input.shortcuts);
    SettingsSnapshot {
        shared,
        local: HostBackendLocalSettings::default(),
        app_state: DesktopAppState::default(),
    }
}

/// Copy one reset scope's backing fields wholesale from the defaults.
fn apply_reset_scope(
    draft: &mut SettingsSnapshot,
    defaults: &SettingsSnapshot,
    scope: &ResetScope,
) {
    match scope {
        ResetScope::All => {
            // Window geometry and the last ROM directory are not settings
            // rows; keep them while everything else returns to defaults.
            let window_sizes = draft.app_state.window_sizes.clone();
            let last_successful_rom_directory =
                draft.app_state.last_successful_rom_directory.clone();
            *draft = defaults.clone();
            draft.app_state.window_sizes = window_sizes;
            draft.app_state.last_successful_rom_directory = last_successful_rom_directory;
        }
        ResetScope::Shortcuts => {
            draft.shared.input.shortcuts = defaults.shared.input.shortcuts.clone();
        }
    }
}

/// Validation issues rendered in the draft language at the call site.
/// The view model only carries structured kinds; text resolves here
/// against the terminal (`SystemDefault` follows the OS locale).
fn issues_text(choice: AppLanguage, issues: &[ValidationIssue]) -> String {
    let language = crate::display_language(choice);
    issues
        .iter()
        .map(|issue| issue.message(language))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExitChoice {
    Editing,
    ConfirmLocal,
    ConfirmGlobal,
    DiscardApproved,
}

#[derive(Debug)]
struct FilesystemValidator;
impl StoragePathValidator for FilesystemValidator {
    fn validate(&self, path: &Path) -> Result<(), StoragePathError> {
        validate_directory_path_typed(path).map_err(|error| match error {
            DirectoryValidationError::NotDirectory => StoragePathError::NotDirectory,
            DirectoryValidationError::Inaccessible(error) => {
                StoragePathError::Inaccessible(error.to_string())
            }
        })
    }
}

pub(crate) struct Editor {
    vm: SettingsViewModel,
    executor: Rc<dyn Executor>,
    baseline: Rc<RefCell<SettingsSnapshot>>,
    /// Canonical defaults for the two reset buttons, built once from
    /// the installed factories (mirrors the Android reset source).
    defaults: SettingsSnapshot,
    /// Enumerated gamepad count at dialog open. Zero until the gamepad
    /// backend connects its first pad; hotplug while open does not refresh.
    pad_count: usize,
    /// Session-order USB vendors snapshotted with the count above: entry
    /// `i` names pad `i`'s button caps while the dialog is open.
    pad_vendors: Vec<Option<u16>>,
    closed: Rc<Cell<bool>>,
    status: Store<String>,
    page: Store<String>,
    general_page: Store<String>,
    controllers_page: Store<String>,
    systems_page: Store<String>,
    gamepads_page: Store<String>,
    exit: Rc<Cell<ExitChoice>>,
    close_prompt: Store<ExitChoice>,
}

impl Editor {
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        adapter: &crate::SessionAdapter,
        executor: Rc<dyn Executor>,
    ) -> Result<Rc<Self>, String> {
        let pad_vendors = adapter.pad_vendors();
        let pad_layouts = nerust_input_traits::PadLayout::for_vendors(&pad_vendors);
        let vm = SettingsViewModel::new(
            snapshot.clone(),
            adapter.factories().to_vec(),
            adapter.supported_sample_rates(),
            Rc::new(FilesystemValidator),
            crate::terminal_language(),
            pad_layouts,
        )
        .map_err(|error| error.to_string())?;
        let defaults = default_snapshot(adapter.factories());
        let pad_count = adapter.gamepad_count();
        Ok(Rc::new(Self {
            vm,
            baseline: Rc::new(RefCell::new(snapshot)),
            defaults,
            pad_count,
            pad_vendors,
            closed: Rc::new(Cell::new(false)),
            status: Store::new(String::new(), executor.clone()),
            page: Store::new("general".into(), executor.clone()),
            general_page: Store::new("app".into(), executor.clone()),
            controllers_page: Store::new(String::new(), executor.clone()),
            systems_page: Store::new(String::new(), executor.clone()),
            gamepads_page: Store::new(String::new(), executor.clone()),
            exit: Rc::new(Cell::new(ExitChoice::Editing)),
            close_prompt: Store::new(ExitChoice::Editing, executor.clone()),
            executor,
        }))
    }
    pub(crate) fn dirty(&self) -> bool {
        self.exit.get() != ExitChoice::DiscardApproved
            && self.vm.snapshot() != *self.baseline.borrow()
    }
    pub(crate) fn confirm_close(&self, global: bool) -> bool {
        if !self.dirty() {
            return true;
        }
        let choice = if global {
            ExitChoice::ConfirmGlobal
        } else {
            ExitChoice::ConfirmLocal
        };
        self.exit.set(choice);
        let _ = self.close_prompt.update(move |prompt| *prompt = choice);
        false
    }
    pub(crate) fn cancel_close(&self) {
        if self.exit.get() == ExitChoice::DiscardApproved {
            self.exit.set(ExitChoice::Editing);
            let _ = self
                .close_prompt
                .update(|prompt| *prompt = ExitChoice::Editing);
        }
    }

    /// Pad capture is armed while a pad binding row waits for a press.
    /// The frame loop polls the live gamepad for exactly this case;
    /// keyboard rows keep their key-event path.
    pub(crate) fn pad_capture_armed(&self) -> bool {
        matches!(
            self.vm.capture.view.get().target,
            Some(CaptureTarget::PadBinding { .. })
        )
    }

    /// Complete an armed pad capture with the first pressed button, then
    /// revalidate so the row leaves its capturing state in one tick.
    pub(crate) fn apply_captured_pad(&self, button: nerust_input_traits::StandardButton) {
        self.vm.capture.apply_captured_pad(button);
        self.validation_status();
    }
    pub(crate) fn close(&self) {
        self.closed.set(true);
        self.vm.capture.cancel_capture();
        self.status.close();
        self.page.close();
        self.general_page.close();
        self.controllers_page.close();
        self.systems_page.close();
        self.gamepads_page.close();
        self.close_prompt.close();
    }
    fn validation_status(&self) {
        let language = self.vm.general.view.get().language;
        let message = self
            .vm
            .finish()
            .err()
            .map(|validation| issues_text(language, &validation.issues))
            .unwrap_or_default();
        let _ = self.status.update(move |status| *status = message);
    }
    fn binding<P: Clone + PartialEq + 'static, T: Clone + PartialEq + 'static>(
        &self,
        property: ReadOnlyObservableProperty<P>,
        forward: impl Fn(&P) -> T + 'static,
        setter: Option<Setter<T>>,
    ) -> Binding<T> {
        let forward = Rc::new(forward);
        let read = property.clone();
        let get = forward.clone();
        let closed = self.closed.clone();
        let vm = self.vm.clone();
        let general = self.vm.general.clone();
        let status = self.status.clone();
        let write = setter.map(|setter| {
            let writer: Rc<dyn Fn(T) -> Result<(), String>> = Rc::new(move |value| {
                if closed.get() {
                    return Err("settings editor is closed".into());
                }
                let result = setter(value).map_err(|error| error.to_string());
                let language = general.view.get().language;
                let message = match &result {
                    Err(error) => error.clone(),
                    Ok(()) => match vm.finish() {
                        Err(validation) => issues_text(language, &validation.issues),
                        Ok(_) => String::new(),
                    },
                };
                let _ = status.update(move |status| *status = message);
                result
            });
            writer
        });
        Binding::external(
            move || get(&read.get()),
            write,
            move |callback| {
                let notify = callback.clone();
                let get = forward.clone();
                let token = property.observe(move |value| notify(get(value)));
                callback(forward(&property.get()));
                Subscription::new(move || drop(token))
            },
        )
        .distinct()
    }
    /// Localized label that follows the draft language live: changing the
    /// UI language in the dialog re-renders every bound caption without
    /// reopening. Choice items already localize this way in the viewmodel;
    /// these cover the static YAML literals.
    pub(super) fn tr(&self, key: UiText) -> Binding<String> {
        let general = self.vm.general.clone();
        self.binding(
            general.view.clone(),
            move |value| I18n::new(crate::display_language(value.language)).text(key),
            None,
        )
    }
    fn language(&self) -> Binding<AppLanguage> {
        let general = self.vm.general.clone();
        self.binding(general.view.clone(), |value| value.language, None)
    }
    fn apply(&self, apply: &Apply) -> Result<(), String> {
        let language = self.vm.general.view.get().language;
        let result = self
            .vm
            .finish()
            .map_err(|validation| issues_text(language, &validation.issues))
            .and_then(|snapshot| {
                apply(snapshot.clone())?;
                self.baseline.replace(snapshot);
                Ok(())
            });
        let message = result.as_ref().map_or_else(Clone::clone, |_| String::new());
        let _ = self.status.update(move |status| *status = message);
        result
    }
    /// Footer and input-row commands, split from `build` so command
    /// wiring evolves apart from binding assembly (see `Commands`).
    fn commands(
        self: &Rc<Self>,
        apply: &Apply,
        quit: Rc<dyn Fn()>,
        view_slot: &Rc<RefCell<Option<WeakView>>>,
    ) -> Commands {
        let commit = apply.clone();
        let exit = self.exit.clone();
        let close = view_slot.clone();
        let discard = self.close_prompt.command(
            |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
            move |_, ()| {
                let global = exit.get() == ExitChoice::ConfirmGlobal;
                exit.set(ExitChoice::DiscardApproved);
                if global {
                    quit();
                } else if let Some(view) = close.borrow().as_ref().and_then(WeakView::upgrade) {
                    view.close();
                }
            },
            Binding::constant(()),
        );
        let exit = self.exit.clone();
        let prompt = self.close_prompt.clone();
        let keep = self.close_prompt.command(
            |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
            move |_, ()| {
                exit.set(ExitChoice::Editing);
                let _ = prompt.update(|prompt| *prompt = ExitChoice::Editing);
            },
            Binding::constant(()),
        );
        let exit = self.exit.clone();
        let close = view_slot.clone();
        let accepted = self.clone();
        let cancel = self.status.command(
            move |_| !accepted.closed.get() && accepted.exit.get() == ExitChoice::Editing,
            move |_, ()| {
                exit.set(ExitChoice::DiscardApproved);
                if let Some(view) = close.borrow().as_ref().and_then(WeakView::upgrade) {
                    view.close();
                }
            },
            Binding::constant(()),
        );
        let editor = self.clone();
        let accepted = self.clone();
        let close = view_slot.clone();
        let accept = self.status.command(
            move |_| {
                !accepted.closed.get()
                    && accepted.exit.get() == ExitChoice::Editing
                    && accepted.vm.finish().is_ok()
            },
            move |_, ()| {
                if editor.apply(&commit).is_ok() {
                    editor.exit.set(ExitChoice::DiscardApproved);
                    if let Some(view) = close.borrow().as_ref().and_then(WeakView::upgrade) {
                        view.close();
                    }
                }
            },
            Binding::constant(()),
        );
        let editor = self.clone();
        let capture: layout::Capture = Rc::new(move |target| {
            let vm = editor.vm.capture.clone();
            let status = editor.status.clone();
            let closed = editor.closed.clone();
            editor.status.command(
                move |_| !closed.get(),
                move |_, ()| {
                    let message = vm
                        .start_capture(target.clone())
                        .map(|_| vm.view.get().prompt)
                        .unwrap_or_else(|error| error.to_string());
                    let _ = status.update(move |status| *status = message);
                },
                Binding::constant(()),
            )
        });
        let editor = self.clone();
        let clear: layout::Capture = Rc::new(move |target| {
            let vm = editor.vm.capture.clone();
            let status = editor.status.clone();
            let closed = editor.closed.clone();
            editor.status.command(
                move |_| !closed.get(),
                move |_, ()| {
                    let message = vm
                        .clear_binding(&target)
                        .map(|_| String::new())
                        .unwrap_or_else(|error| error.to_string());
                    let _ = status.update(move |status| *status = message);
                },
                Binding::constant(()),
            )
        });
        let editor = self.clone();
        let reset: layout::Reset = Rc::new(move |scope| {
            let vm = editor.vm.clone();
            let general = editor.vm.general.clone();
            let status = editor.status.clone();
            let closed = editor.closed.clone();
            let defaults = editor.defaults.clone();
            editor.status.command(
                move |_| !closed.get(),
                move |_, ()| {
                    let result = vm.restore_scope(&defaults, |draft, defaults| {
                        apply_reset_scope(draft, defaults, &scope);
                    });
                    let message = match &result {
                        Err(error) => error.to_string(),
                        Ok(()) => {
                            let language = general.view.get().language;
                            match vm.finish() {
                                Err(validation) => issues_text(language, &validation.issues),
                                Ok(_) => String::new(),
                            }
                        }
                    };
                    let _ = status.update(move |status| *status = message);
                },
                Binding::constant(()),
            )
        });
        Commands {
            cancel,
            accept,
            discard,
            keep,
            capture,
            clear,
            reset,
        }
    }

    /// All dialog bindings, split from `build`; `try_view!` consumes
    /// the result directly.
    fn presentation(self: &Rc<Self>, commands: Commands) -> Presentation {
        macro_rules! field {
            ($section:ident, $field:ident, $setter:ident) => {{
                let vm = self.vm.$section.clone();
                self.binding(
                    vm.view.clone(),
                    |value| value.$field.clone(),
                    Some(Rc::new(move |value| vm.$setter(value))),
                )
            }};
        }
        macro_rules! choices {
            ($section:ident, $field:ident, $choices:ident, $setter:ident) => {{
                choice(
                    self.binding(
                        self.vm.$section.view.clone(),
                        |value| value.$choices.clone(),
                        None,
                    ),
                    field!($section, $field, $setter),
                )
            }};
        }
        let vm = self.vm.general.clone();
        let directory = self.binding(
            vm.view.clone(),
            |value| value.storage_directory.clone(),
            Some(Rc::new(move |value: String| {
                vm.set_storage_directory(if value.is_empty() {
                    None
                } else {
                    Some(value.into())
                })
            })),
        );
        let volume = field!(audio, volume_percent, set_volume).convert(f64::from, |value| {
            checked_integer(value, 0.0, 100.0).map(|value| value as u8)
        });
        let latency = field!(audio, latency_ms, set_latency).convert(f64::from, |value| {
            checked_integer(value, 10.0, 200.0).map(|value| value as u16)
        });
        let language = self.language();
        let rumble_motion = field!(rumble_motion, motion_enabled, set_motion_enabled);
        let rumble_enabled = field!(rumble_motion, rumble_enabled, set_rumble_enabled);
        let rumble_strength = field!(rumble_motion, rumble_strength_percent, set_rumble_strength)
            .convert(f64::from, |value| {
                checked_integer(value, 0.0, 100.0).map(|value| value as u8)
            });
        let rumble_strength_label = language.clone().zip(&rumble_strength).map(|(lang, value)| {
            format!(
                "{}: {value:.0}%",
                I18n::new(crate::display_language(lang)).text(UiText::RumbleStrength)
            )
        });
        let rumble_target = choices!(
            rumble_motion,
            rumble_target,
            rumble_target_choices,
            set_rumble_target
        );
        let Commands {
            cancel,
            accept,
            discard,
            keep,
            capture,
            clear,
            reset,
        } = commands;
        let executor = self.executor.clone();
        let mk_reset =
            |key: &str, scope: ResetScope, label: Binding<String>| -> Keyed<layout::ResetRow> {
                Keyed::new(
                    key.to_string(),
                    Store::new(
                        layout::ResetRow {
                            label,
                            reset: reset(scope),
                        },
                        executor.clone(),
                    ),
                )
            };
        let mk_choice = |key: &str,
                         label: Binding<String>,
                         choice: ChoiceBinding|
         -> Option<Keyed<layout::ChoiceRow>> {
            Some(Keyed::new(
                key.to_string(),
                Store::new(
                    layout::ChoiceRow {
                        label,
                        choices: choice.items,
                        selected: choice.selected,
                        enabled: Binding::constant(true),
                    },
                    executor.clone(),
                ),
            ))
        };
        let mk_check = |key: &str,
                        label: Binding<String>,
                        checked: Binding<bool>|
         -> Option<Keyed<layout::CheckRow>> {
            Some(Keyed::new(
                key.to_string(),
                Store::new(layout::CheckRow { label, checked }, executor.clone()),
            ))
        };
        let language_choice = choices!(general, language, language_choices, set_language);
        let storage_choice = choices!(
            general,
            storage_policy,
            storage_policy_choices,
            set_storage_policy
        );
        let directory_visible = self.binding(
            self.vm.general.view.clone(),
            |value| value.show_storage_directory,
            None,
        );
        let fullscreen = field!(video, fullscreen_default, set_fullscreen_default);
        let vsync = field!(video, vsync, set_vsync);
        let scaling = choices!(video, scaling, scaling_choices, set_scaling);
        let muted = field!(audio, muted, set_mute);
        let volume_label = language.clone().zip(&volume).map(|(lang, value)| {
            format!(
                "{}: {value:.0}%",
                I18n::new(crate::display_language(lang)).text(UiText::MasterVolume)
            )
        });
        let latency_label = language.clone().zip(&latency).map(|(lang, value)| {
            format!(
                "{}: {value:.0} ms",
                I18n::new(crate::display_language(lang)).text(UiText::Latency)
            )
        });
        let sample_rate = choices!(audio, sample_rate, sample_rate_choices, set_sample_rate);
        let general_tabs: Vec<Keyed<layout::GeneralTab>> = ["app", "video", "audio", "rumble"]
            .into_iter()
            .map(|key| {
                let show = |id: &str| key == id;
                let tab_label = match key {
                    "app" => self.tr(UiText::SectionApp).get(),
                    "video" => self.tr(UiText::Video).get(),
                    "audio" => self.tr(UiText::Audio).get(),
                    _ => self.tr(UiText::RumbleMotion).get(),
                };
                // Only App carries a reset (whole-settings); the other
                // tabs own no reset.
                let resets = match key {
                    "app" => vec![mk_reset(
                        "reset-all",
                        ResetScope::All,
                        self.tr(UiText::ResetSettings),
                    )],
                    _ => Vec::new(),
                };
                Keyed::new(
                    key.to_string(),
                    Store::new(
                        layout::GeneralTab {
                            label: tab_label,
                            show_app: show("app"),
                            show_video: show("video"),
                            show_audio: show("audio"),
                            show_rumble: show("rumble"),
                            language: mk_choice(
                                "language",
                                self.tr(UiText::Language),
                                language_choice.clone(),
                            ),
                            storage: mk_choice(
                                "storage",
                                self.tr(UiText::SaveStoragePolicy),
                                storage_choice.clone(),
                            ),
                            directory: directory.clone(),
                            directory_visible: directory_visible.clone(),
                            fullscreen: mk_check(
                                "fullscreen",
                                self.tr(UiText::FullscreenDefault),
                                fullscreen.clone(),
                            ),
                            vsync: mk_check("vsync", self.tr(UiText::Vsync), vsync.clone()),
                            scaling: mk_choice(
                                "scaling",
                                self.tr(UiText::Scaling),
                                scaling.clone(),
                            ),
                            muted: mk_check("muted", self.tr(UiText::Mute), muted.clone()),
                            volume: volume.clone(),
                            volume_label: volume_label.clone(),
                            latency: latency.clone(),
                            latency_label: latency_label.clone(),
                            sample_rate: mk_choice(
                                "sample-rate",
                                self.tr(UiText::SampleRate),
                                sample_rate.clone(),
                            ),
                            rumble_motion: mk_check(
                                "rumble-motion",
                                self.tr(UiText::MotionSensor),
                                rumble_motion.clone(),
                            ),
                            rumble_enabled: mk_check(
                                "rumble-enabled",
                                self.tr(UiText::Rumble),
                                rumble_enabled.clone(),
                            ),
                            rumble_strength: rumble_strength.clone(),
                            rumble_strength_label: rumble_strength_label.clone(),
                            rumble_target: mk_choice(
                                "rumble-target",
                                self.tr(UiText::RumbleTarget),
                                rumble_target.clone(),
                            ),
                            resets,
                        },
                        executor.clone(),
                    ),
                )
            })
            .collect();
        Presentation {
            general_tabs: Binding::constant(general_tabs),
            general_selection: self
                .general_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            controllers_pages: Binding::constant(layout::controllers_pages(self, &capture, &clear)),
            controllers_selection: self
                .controllers_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            systems_pages: Binding::constant(layout::systems_pages(self)),
            systems_selection: self
                .systems_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            gamepads_blocks: Binding::constant(layout::gamepads_blocks(
                self,
                &capture,
                &clear,
                self.pad_count,
            )),
            gamepads_selection: self
                .gamepads_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            shortcuts_page: layout::shortcuts_page(self, &capture, &clear),
            shortcuts_resets: Binding::constant(vec![mk_reset(
                "reset",
                ResetScope::Shortcuts,
                self.tr(UiText::ResetScreen),
            )]),
            page: self
                .page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            status: self.status.bind(Clone::clone, None),
            confirmation: self.close_prompt.bind(
                |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
                None,
            ),
            info_enabled: field!(notifications, info_enabled, set_info_enabled),
            info_duration: integer_text(field!(
                notifications,
                info_duration_ms,
                set_info_duration_ms
            )),
            info_color: choices!(notifications, info_color, color_choices, set_info_color),
            warning_enabled: field!(notifications, warning_enabled, set_warning_enabled),
            warning_duration: integer_text(field!(
                notifications,
                warning_duration_ms,
                set_warning_duration_ms
            )),
            warning_color: choices!(
                notifications,
                warning_color,
                color_choices,
                set_warning_color
            ),
            error_enabled: field!(notifications, error_enabled, set_error_enabled),
            error_modal: field!(notifications, error_modal, set_error_modal),
            error_color: choices!(notifications, error_color, color_choices, set_error_color),
            status_enabled: field!(notifications, status_enabled, set_status_enabled),
            position: choices!(notifications, position, position_choices, set_position),
            status_position: choices!(
                notifications,
                status_position,
                position_choices,
                set_status_position
            ),
            area: choices!(notifications, area_mode, area_mode_choices, set_area_mode),
            status_area: choices!(
                notifications,
                status_area_mode,
                area_mode_choices,
                set_status_area_mode
            ),
            // Same vertical edge: the transient area mode governs the edge
            // (see `OsdChrome::reserved_rows`), so the status picker must
            // not accept a divergent mode. The combo is disabled and a
            // follow note shows instead — same contract as the old Tao
            // note and the GTK insensitive picker before the YAML move.
            status_area_enabled: self.binding(
                self.vm.notifications.view.clone(),
                |value| !value.position.same_edge(value.status_position),
                None,
            ),
            status_area_follows: self.binding(
                self.vm.notifications.view.clone(),
                |value| value.position.same_edge(value.status_position),
                None,
            ),
            labels: Labels {
                title: self.binding(
                    self.vm.general.view.clone(),
                    |value| {
                        format!(
                            "nerust {}",
                            I18n::new(crate::display_language(value.language))
                                .text(UiText::Settings)
                        )
                    },
                    None,
                ),
                tab_general: self.tr(UiText::General),
                tab_controllers: self.tr(UiText::SectionControllers),
                tab_gamepads: self.tr(UiText::Gamepads),
                tab_shortcuts: self.tr(UiText::Shortcuts),
                tab_notifications: self.tr(UiText::Notifications),
                tab_advanced: self.tr(UiText::SectionAdvanced),
                info: self.tr(UiText::NotificationInfo),
                info_enabled: self.tr(UiText::NotificationEnabled),
                info_duration: self.tr(UiText::NotificationDuration),
                info_color: self.tr(UiText::NotificationColor),
                warning: self.tr(UiText::NotificationWarning),
                warning_enabled: self.tr(UiText::NotificationEnabled),
                warning_duration: self.tr(UiText::NotificationDuration),
                warning_color: self.tr(UiText::NotificationColor),
                error: self.tr(UiText::NotificationError),
                error_enabled: self.tr(UiText::NotificationErrorOnScreen),
                error_modal: self.tr(UiText::NotificationErrorModal),
                error_color: self.tr(UiText::NotificationColor),
                display_section: self.tr(UiText::NotificationSectionDisplay),
                status_section: self.tr(UiText::NotificationSectionStatus),
                status_enabled: self.tr(UiText::NotificationEnabled),
                follow_note: self.tr(UiText::NotificationStatusAreaFollows),
                cancel: self.tr(UiText::Cancel),
                save: self.tr(UiText::Save),
                confirm: self.tr(UiText::DiscardUnsavedChanges),
                discard: self.tr(UiText::Discard),
                keep: self.tr(UiText::KeepEditing),
            },
            cancel,
            accept,
            discard,
            keep,
        }
    }

    pub(crate) fn build(self: &Rc<Self>, apply: Apply, quit: Rc<dyn Fn()>) -> Result<View, String> {
        let view_slot = Rc::new(RefCell::new(None::<WeakView>));
        let commands = self.commands(&apply, quit, &view_slot);
        let keys = commands.clone();
        let bindings = self.presentation(commands);
        let view = panechord::try_view!("ui/settings.yaml", bindings, panechord::backend)?;
        let editor = self.clone();
        view.on_surface_event(Rc::new(move |event| {
            if editor.closed.get() {
                return;
            }
            if let panechord::WindowEvent::KeyboardInput {
                key,
                pressed: true,
                repeat: false,
                synthetic: false,
            } = event
            {
                // Capture sessions own the keyboard: Escape aborts the capture
                // and any other key becomes the candidate binding.
                if editor.vm.capture.view.get().target.is_some() {
                    if key.as_str() == "Escape" {
                        editor.vm.capture.cancel_capture();
                    } else if let Some(key) = crate::SessionAdapter::key(key) {
                        editor.vm.capture.apply_captured_key(key);
                    } else {
                        return;
                    }
                    editor.validation_status();
                    return;
                }
                // Framework footers own the visual order; the dialog owner
                // owns the keys. Confirmation keeps the safe choice on both
                // Escape and Return; `invoke` no-ops while disabled.
                let confirming = editor.close_prompt.with(|prompt| {
                    matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal)
                });
                if key.as_str() == "Escape" {
                    let _ = if confirming {
                        keys.keep.invoke()
                    } else {
                        keys.cancel.invoke()
                    };
                    editor.validation_status();
                } else if key.as_str() == "Enter" || key.as_str() == "NumpadEnter" {
                    let _ = if confirming {
                        keys.keep.invoke()
                    } else {
                        keys.accept.invoke()
                    };
                    editor.validation_status();
                }
            }
        }));
        view_slot.replace(Some(view.downgrade()));
        Ok(view)
    }
}

#[derive(Clone)]
struct ChoiceBinding {
    items: Binding<Vec<Choice>>,
    selected: Binding<String>,
}

/// Footer, input-row, and reset commands assembled by `Editor::commands`.
/// Kept at module scope (items are not allowed inside `impl` blocks).
/// Cloned into the surface key handler so Return/Escape honor the same
/// commands as the footer buttons, with capture sessions first.
#[derive(Clone)]
struct Commands {
    cancel: Command,
    accept: Command,
    discard: Command,
    keep: Command,
    capture: layout::Capture,
    clear: layout::Capture,
    reset: layout::Reset,
}

/// Static YAML captions as language-following bindings. Literals never
/// update; every settings caption rides the draft language through these
/// so switching UI language re-renders the open dialog in place.
#[derive(Clone)]
struct Labels {
    title: Binding<String>,
    tab_general: Binding<String>,
    tab_controllers: Binding<String>,
    tab_gamepads: Binding<String>,
    tab_shortcuts: Binding<String>,
    tab_notifications: Binding<String>,
    tab_advanced: Binding<String>,
    info: Binding<String>,
    info_enabled: Binding<String>,
    info_duration: Binding<String>,
    info_color: Binding<String>,
    warning: Binding<String>,
    warning_enabled: Binding<String>,
    warning_duration: Binding<String>,
    warning_color: Binding<String>,
    error: Binding<String>,
    error_enabled: Binding<String>,
    error_modal: Binding<String>,
    error_color: Binding<String>,
    display_section: Binding<String>,
    status_section: Binding<String>,
    status_enabled: Binding<String>,
    follow_note: Binding<String>,
    cancel: Binding<String>,
    save: Binding<String>,
    confirm: Binding<String>,
    discard: Binding<String>,
    keep: Binding<String>,
}
fn choice<T: Clone + Eq + 'static>(
    choices: Binding<Vec<ChoiceView<T>>>,
    selected: Binding<T>,
) -> ChoiceBinding {
    let read = choices.clone();
    let write = choices.clone();
    ChoiceBinding {
        items: choices.map(|choices| {
            choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| Choice {
                    id: index.to_string(),
                    label: choice.label,
                })
                .collect()
        }),
        selected: selected.convert(
            move |value| {
                choice_position(&read.get(), &value)
                    .map(|index| index.to_string())
                    .unwrap_or_default()
            },
            move |id| {
                id.parse::<usize>()
                    .ok()
                    .and_then(|index| write.get().get(index).map(|choice| choice.value.clone()))
                    .ok_or_else(|| "unknown choice".into())
            },
        ),
    }
}
fn checked_integer(value: f64, min: f64, max: f64) -> Result<u32, String> {
    if !value.is_finite() || value < min || value > max || value.fract() != 0.0 {
        Err("value must be an integer within range".into())
    } else {
        Ok(value as u32)
    }
}
fn integer_text(binding: Binding<u32>) -> Binding<String> {
    binding.convert(
        |value| value.to_string(),
        |value| {
            value
                .parse()
                .map_err(|_| "duration must be a nonnegative integer".into())
        },
    )
}

#[derive(Clone)]
struct Presentation {
    general_tabs: Binding<Vec<panechord::Keyed<layout::GeneralTab>>>,
    general_selection: Binding<String>,
    controllers_pages: Binding<Vec<panechord::Keyed<layout::ControllerPage>>>,
    controllers_selection: Binding<String>,
    systems_pages: Binding<Vec<panechord::Keyed<layout::SystemPage>>>,
    systems_selection: Binding<String>,
    gamepads_blocks: Binding<Vec<panechord::Keyed<layout::GamepadBlock>>>,
    gamepads_selection: Binding<String>,
    shortcuts_page: Binding<Vec<Keyed<layout::BindingPage>>>,
    shortcuts_resets: Binding<Vec<Keyed<layout::ResetRow>>>,
    page: Binding<String>,
    status: Binding<String>,
    confirmation: Binding<bool>,
    info_enabled: Binding<bool>,
    info_duration: Binding<String>,
    info_color: ChoiceBinding,
    warning_enabled: Binding<bool>,
    warning_duration: Binding<String>,
    warning_color: ChoiceBinding,
    error_enabled: Binding<bool>,
    error_modal: Binding<bool>,
    error_color: ChoiceBinding,
    status_enabled: Binding<bool>,
    position: ChoiceBinding,
    status_position: ChoiceBinding,
    area: ChoiceBinding,
    status_area: ChoiceBinding,
    status_area_enabled: Binding<bool>,
    status_area_follows: Binding<bool>,
    labels: Labels,
    cancel: Command,
    accept: Command,
    discard: Command,
    keep: Command,
}
impl Presentation {
    fn command_cancel(&self, _: Binding<String>) -> Command {
        self.cancel.clone()
    }
    fn command_accept(&self, _: Binding<String>) -> Command {
        self.accept.clone()
    }
    fn command_discard(&self, _: Binding<String>) -> Command {
        self.discard.clone()
    }
    fn command_keep(&self, _: Binding<String>) -> Command {
        self.keep.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_binding_rejects_nonfinite_fractional_and_out_of_range_values() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 101.0, 1.5] {
            assert!(checked_integer(value, 0.0, 100.0).is_err());
        }
        assert_eq!(checked_integer(0.0, 0.0, 100.0), Ok(0));
        assert_eq!(checked_integer(100.0, 0.0, 100.0), Ok(100));
    }

    struct TestExecutor;
    impl Executor for TestExecutor {
        fn spawn(&self, _: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
            panic!("settings validation must not dispatch async work");
        }
    }

    /// System choice save/reopen through a real adapter: change a choice on
    /// the live factories, finish to a snapshot, and reopen from that
    /// snapshot. The RPA native review covers the same topology on glass;
    /// this pins the editor/owner wiring headlessly.
    #[test]
    fn system_choice_save_and_reopen_round_trip() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        assert!(!editor.dirty());
        assert!(editor.confirm_close(false));
        let systems = editor.vm.systems();
        assert!(!systems.is_empty(), "real registry exposes systems");
        let (system, field, other) = systems
            .iter()
            .flat_map(|system| {
                let view = system.view.get();
                view.fields
                    .into_iter()
                    .filter_map(|field| {
                        let id = field.id.clone();
                        field
                            .choices
                            .iter()
                            .find(|choice| choice.value != field.selected)
                            .map(|choice| (system, id, choice.value.clone()))
                    })
                    .collect::<Vec<_>>()
            })
            .next()
            .expect("a system choice has an alternative");
        system.set_choice(&field, &other).expect("choice applies");
        assert!(editor.dirty());
        assert!(!editor.confirm_close(false));
        let snapshot = editor.vm.finish().expect("choice is valid");
        let reopened = Editor::new(snapshot, &adapter, Rc::new(TestExecutor))
            .expect("reopen builds on real factories");
        assert!(!reopened.dirty());
        let selected = reopened.vm.systems().iter().find_map(|system| {
            system
                .view
                .get()
                .fields
                .into_iter()
                .find(|candidate| candidate.id == field)
                .map(|candidate| candidate.selected)
        });
        assert_eq!(selected.as_ref(), Some(&other));
        assert!(reopened.confirm_close(false));
    }

    /// The App-level reset-all restores every default except window
    /// geometry and the last ROM directory (which are not settings rows).
    #[test]
    fn reset_all_restores_defaults_but_keeps_window_geometry() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let before = editor.vm.snapshot();
        editor.vm.audio.set_volume(11).expect("volume applies");
        assert_ne!(editor.vm.snapshot(), before, "draft diverges");
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let view_slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&apply, Rc::new(|| {}), &view_slot);
        let _ = (commands.reset)(ResetScope::All).invoke();
        let after = editor.vm.snapshot();
        let defaults = default_snapshot(adapter.factories());
        assert_eq!(after.shared, defaults.shared);
        assert_eq!(after.local, defaults.local);
        let mut expected_app = defaults.app_state.clone();
        expected_app.window_sizes = before.app_state.window_sizes.clone();
        expected_app.last_successful_rom_directory =
            before.app_state.last_successful_rom_directory.clone();
        assert_eq!(after.app_state, expected_app);
    }

    /// Same-edge gating for the status area picker: when notification and
    /// status share one horizontal band, the transient area mode governs
    /// the edge (`OsdChrome::reserved_rows`), so the status picker must
    /// disable and the follow note must show. Split edges re-enable it.
    #[test]
    fn status_area_picker_disables_on_same_edge() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::model::local::NotificationPosition;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        // Drive the production presentation bindings, not local
        // re-implementations: these are the exact bindings the YAML footer
        // consumes, so breaking the wiring fails the test.
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let commands = editor.commands(&apply, Rc::new(|| {}), &view_slot);
        let presentation = editor.presentation(commands);
        let notifications = editor.vm.notifications.clone();
        // Default: BottomLeft vs TopLeft — split edges, picker live.
        assert!(!presentation.status_area_follows.get());
        assert!(presentation.status_area_enabled.get());
        // Same band: BottomLeft vs BottomRight — picker disabled, note shown.
        notifications
            .set_status_position(NotificationPosition::BottomRight)
            .expect("position applies");
        assert!(presentation.status_area_follows.get());
        assert!(!presentation.status_area_enabled.get());
        // Back to split: TopRight vs BottomRight — picker live again.
        notifications
            .set_status_position(NotificationPosition::TopRight)
            .expect("position applies");
        assert!(!presentation.status_area_follows.get());
        assert!(presentation.status_area_enabled.get());
    }

    /// Settings captions follow the draft language live: switching to
    /// Japanese re-renders every bound label in the open dialog, so no
    /// caption may stay a static literal.
    #[test]
    fn settings_labels_follow_draft_language() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::model::language::AppLanguage;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let general = editor.tr(UiText::General);
        let enabled = editor.tr(UiText::NotificationEnabled);
        assert_eq!(general.get(), "General");
        assert_eq!(enabled.get(), "Enabled");
        editor
            .vm
            .general
            .set_language(AppLanguage::Japanese)
            .expect("language applies");
        assert_eq!(general.get(), "一般");
        assert_eq!(enabled.get(), "有効");
        assert_eq!(
            editor.tr(UiText::DiscardUnsavedChanges).get(),
            "保存されていない変更を破棄しますか？"
        );
        editor
            .vm
            .general
            .set_language(AppLanguage::English)
            .expect("language applies");
        assert_eq!(general.get(), "General");
    }

    /// Filesystem backing for the validator: real directories (and
    /// missing leaves beneath them) validate; plain files do not.
    #[test]
    fn filesystem_validator_accepts_dirs_and_rejects_files() {
        let root =
            std::env::temp_dir().join(format!("nerust-settings-validator-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("validator temp root");
        let validator = FilesystemValidator;
        assert!(validator.validate(root.as_path()).is_ok());
        assert!(validator.validate(&root.join("missing-leaf")).is_ok());
        let file = root.join("save.bin");
        std::fs::write(&file, b"fixture").expect("validator temp file");
        assert!(matches!(
            validator.validate(&file),
            Err(StoragePathError::NotDirectory)
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Exit transitions without a backend view: clean drafts close
    /// silently, dirty drafts prompt, and close aborts any capture.
    #[test]
    fn confirm_close_cancel_close_and_close_cover_exit_transitions() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::{editor::CaptureTarget, model::input::ShortcutAction};
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        assert!(editor.confirm_close(false));
        assert!(editor.exit.get() == ExitChoice::Editing);
        let vsync = editor.vm.video.view.get().vsync;
        editor.vm.video.set_vsync(!vsync).expect("vsync applies");
        assert!(editor.dirty());
        assert!(!editor.confirm_close(false));
        assert!(editor.exit.get() == ExitChoice::ConfirmLocal);
        assert!(
            editor
                .close_prompt
                .with(|prompt| *prompt == ExitChoice::ConfirmLocal)
        );
        // A pending prompt is not an approved discard; cancel is a no-op.
        editor.cancel_close();
        assert!(editor.exit.get() == ExitChoice::ConfirmLocal);
        assert!(!editor.confirm_close(true));
        assert!(editor.exit.get() == ExitChoice::ConfirmGlobal);
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        editor
            .vm
            .capture
            .start_capture(target)
            .expect("capture starts");
        editor.close();
        assert!(editor.closed.get());
        assert!(editor.vm.capture.view.get().target.is_none());
    }

    /// The status line reports the joined validation issues and clears
    /// once the draft is valid again.
    #[test]
    fn validation_status_reports_and_clears_issues() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        editor.validation_status();
        assert!(editor.status.with(|status| status.clone()).is_empty());
        editor
            .vm
            .audio
            .set_volume(200)
            .expect("transact accepts out-of-range volume");
        editor.validation_status();
        assert!(
            editor
                .status
                .with(|status| status.clone())
                .contains("Master volume")
        );
        editor.vm.audio.set_volume(50).expect("volume applies");
        editor.validation_status();
        assert!(editor.status.with(|status| status.clone()).is_empty());
    }

    /// Apply re-baselines on success, surfaces backend errors on status,
    /// and never calls the backend for an invalid draft.
    #[test]
    fn apply_persists_baseline_on_success_and_reports_failures() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let seen = Rc::new(RefCell::new(None::<SettingsSnapshot>));
        let apply: Apply = Rc::new({
            let seen = seen.clone();
            move |snapshot: SettingsSnapshot| {
                *seen.borrow_mut() = Some(snapshot);
                Ok(())
            }
        });
        let vsync = editor.vm.video.view.get().vsync;
        editor.vm.video.set_vsync(!vsync).expect("vsync applies");
        assert!(editor.dirty());
        assert!(editor.apply(&apply).is_ok());
        assert!(!editor.dirty());
        assert!(seen.borrow().is_some());
        assert!(editor.status.with(|status| status.clone()).is_empty());
        let failing: Apply = Rc::new(|_| Err("backend down".into()));
        editor.vm.video.set_vsync(vsync).expect("vsync applies");
        assert!(editor.apply(&failing).is_err());
        assert_eq!(editor.status.with(|status| status.clone()), "backend down");
        assert!(editor.dirty());
        let called = Rc::new(Cell::new(false));
        let guarded: Apply = Rc::new({
            let called = called.clone();
            move |_| {
                called.set(true);
                Ok(())
            }
        });
        editor
            .vm
            .audio
            .set_volume(200)
            .expect("transact accepts out-of-range volume");
        assert!(editor.apply(&guarded).is_err());
        assert!(!called.get());
        assert!(
            editor
                .status
                .with(|status| status.clone())
                .contains("Master volume")
        );
    }

    /// Footer commands without a view: cancel approves the discard,
    /// keep resumes editing, and only the global discard quits.
    #[test]
    fn commands_drive_cancel_accept_discard_and_keep() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit = Rc::new(Cell::new(false));
        let quit_fn: Rc<dyn Fn()> = Rc::new({
            let quit = quit.clone();
            move || quit.set(true)
        });
        let commands = editor.commands(&apply, quit_fn, &view_slot);
        assert!(commands.cancel.enabled.get());
        assert!(commands.accept.enabled.get());
        assert!(!commands.discard.enabled.get());
        assert!(!commands.keep.enabled.get());
        let _ = commands.cancel.invoke();
        assert!(editor.exit.get() == ExitChoice::DiscardApproved);
        assert!(!editor.dirty());
        assert!(!commands.cancel.enabled.get());
        editor.cancel_close();
        assert!(editor.exit.get() == ExitChoice::Editing);
        assert!(
            editor
                .close_prompt
                .with(|prompt| *prompt == ExitChoice::Editing)
        );
        editor.cancel_close();
        assert!(editor.exit.get() == ExitChoice::Editing);
        let vsync = editor.vm.video.view.get().vsync;
        editor.vm.video.set_vsync(!vsync).expect("vsync applies");
        assert!(!editor.confirm_close(false));
        assert!(commands.discard.enabled.get());
        assert!(commands.keep.enabled.get());
        let _ = commands.keep.invoke();
        assert!(editor.exit.get() == ExitChoice::Editing);
        assert!(!commands.discard.enabled.get());
        assert!(!editor.confirm_close(false));
        let _ = commands.discard.invoke();
        assert!(editor.exit.get() == ExitChoice::DiscardApproved);
        assert!(!quit.get());
        editor.cancel_close();
        assert!(!editor.confirm_close(true));
        assert!(editor.exit.get() == ExitChoice::ConfirmGlobal);
        let _ = commands.discard.invoke();
        assert!(quit.get());
    }

    /// Accept applies valid drafts, no-ops on invalid ones, and leaves
    /// the dialog open with the backend error on status when saving fails.
    #[test]
    fn accept_applies_on_valid_draft_and_stays_put_otherwise() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let applied = Rc::new(Cell::new(false));
        let apply: Apply = Rc::new({
            let applied = applied.clone();
            move |_| {
                applied.set(true);
                Ok(())
            }
        });
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let view_slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&apply, quit, &view_slot);
        let vsync = editor.vm.video.view.get().vsync;
        editor.vm.video.set_vsync(!vsync).expect("vsync applies");
        assert!(commands.accept.enabled.get());
        let _ = commands.accept.invoke();
        assert!(applied.get());
        assert!(editor.exit.get() == ExitChoice::DiscardApproved);
        assert!(!editor.dirty());
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        editor
            .vm
            .audio
            .set_volume(200)
            .expect("transact accepts out-of-range volume");
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let view_slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&apply, quit, &view_slot);
        assert!(!commands.accept.enabled.get());
        assert!(!commands.accept.invoke());
        assert!(editor.exit.get() == ExitChoice::Editing);
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let vsync = editor.vm.video.view.get().vsync;
        editor.vm.video.set_vsync(!vsync).expect("vsync applies");
        let failing: Apply = Rc::new(|_| Err("backend down".into()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let view_slot = Rc::new(RefCell::new(None));
        let commands = editor.commands(&failing, quit, &view_slot);
        let _ = commands.accept.invoke();
        assert!(editor.exit.get() == ExitChoice::Editing);
        assert_eq!(editor.status.with(|status| status.clone()), "backend down");
    }

    /// Capture/clear commands start and end a session headlessly: the
    /// prompt surfaces on status and clears with the binding.
    #[test]
    fn capture_and_clear_commands_update_prompt_and_status() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::{editor::CaptureTarget, model::input::ShortcutAction};
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let commands = editor.commands(&apply, quit, &view_slot);
        let target = CaptureTarget::Shortcut(ShortcutAction::TogglePause);
        let capture = (commands.capture)(target.clone());
        let clear = (commands.clear)(target.clone());
        assert!(capture.enabled.get());
        let _ = capture.invoke();
        assert!(editor.vm.capture.view.get().target.is_some());
        assert!(!editor.status.with(|status| status.clone()).is_empty());
        let _ = clear.invoke();
        assert!(editor.vm.capture.view.get().target.is_none());
        assert!(editor.status.with(|status| status.clone()).is_empty());
        editor.close();
        assert!(!capture.enabled.get());
        assert!(!capture.invoke());
    }

    /// Binding write paths: setter failures and validation failures both
    /// land on status, and closed editors reject every write.
    #[test]
    fn binding_write_errors_surface_on_status() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let input = editor
            .vm
            .inputs()
            .first()
            .expect("real registry exposes inputs")
            .clone();
        let slot = input
            .view
            .get()
            .slots
            .first()
            .expect("nes exposes a slot")
            .slot_id;
        let writer: Binding<Option<String>> = editor.binding(
            input.view.clone(),
            |_| None,
            Some(Rc::new(move |value: Option<String>| {
                input.set_controller_slot(slot, value.as_deref())
            })),
        );
        assert!(!writer.set(Some("not-a-real-profile".into())));
        assert!(!editor.status.with(|status| status.clone()).is_empty());
        // A passing setter with an invalid draft reports validation.
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let presentation = editor.presentation(editor.commands(&apply, quit, &view_slot));
        editor
            .vm
            .audio
            .set_volume(200)
            .expect("transact accepts out-of-range volume");
        let audio = presentation
            .general_tabs
            .get()
            .into_iter()
            .find(|tab| tab.key == "audio")
            .expect("audio tab builds")
            .model
            .snapshot();
        assert!(
            audio
                .muted
                .as_ref()
                .expect("muted row builds")
                .model
                .snapshot()
                .checked
                .set(true)
        );
        assert!(presentation.status.get().contains("Master volume"));
        editor.close();
        assert!(!writer.set(None));
    }

    /// Presentation bindings mirror the draft and round-trip writes:
    /// numbers convert, choices reposition, durations parse as text.
    /// General L2 rows live on their tab entries now; this test reaches
    /// them through the tab list.
    #[test]
    fn presentation_bindings_mirror_draft_and_round_trip_writes() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::model::language::AppLanguage;
        fn general_tab(presentation: &Presentation, key: &str) -> layout::GeneralTab {
            presentation
                .general_tabs
                .get()
                .into_iter()
                .find(|tab| tab.key == key)
                .unwrap_or_else(|| panic!("{key} tab builds"))
                .model
                .snapshot()
        }
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let presentation = editor.presentation(editor.commands(&apply, quit, &view_slot));
        assert_eq!(presentation.page.get(), "general");
        assert!(presentation.status.get().is_empty());
        assert!(!presentation.confirmation.get());
        // General L2 tabs carry every previous general/video/audio row.
        let app = general_tab(&presentation, "app");
        let video = general_tab(&presentation, "video");
        let audio = general_tab(&presentation, "audio");
        let rumble = general_tab(&presentation, "rumble");
        assert_eq!(
            video
                .fullscreen
                .as_ref()
                .expect("fullscreen row builds")
                .model
                .snapshot()
                .checked
                .get(),
            editor.vm.video.view.get().fullscreen_default
        );
        assert_eq!(
            video
                .vsync
                .as_ref()
                .expect("vsync row builds")
                .model
                .snapshot()
                .checked
                .get(),
            editor.vm.video.view.get().vsync
        );
        assert_eq!(
            audio
                .muted
                .as_ref()
                .expect("muted row builds")
                .model
                .snapshot()
                .checked
                .get(),
            editor.vm.audio.view.get().muted
        );
        assert_eq!(
            audio.volume.get(),
            f64::from(editor.vm.audio.view.get().volume_percent)
        );
        assert_eq!(
            audio.latency.get(),
            f64::from(editor.vm.audio.view.get().latency_ms)
        );
        assert_eq!(
            app.directory.get(),
            editor.vm.general.view.get().storage_directory
        );
        assert_eq!(
            app.directory_visible.get(),
            editor.vm.general.view.get().show_storage_directory
        );
        assert!(
            rumble
                .rumble_motion
                .as_ref()
                .expect("motion row builds")
                .model
                .snapshot()
                .checked
                .get()
        );
        assert!(
            rumble
                .rumble_enabled
                .as_ref()
                .expect("rumble row builds")
                .model
                .snapshot()
                .checked
                .get()
        );
        assert_eq!(rumble.rumble_strength.get(), 100.0);
        assert_eq!(
            presentation.controllers_pages.get().len(),
            editor.vm.inputs().len(),
            "one controllers page per machine"
        );
        assert_eq!(
            presentation.systems_pages.get().len(),
            editor.vm.systems().len(),
            "one system page per machine"
        );
        assert!(
            presentation.gamepads_blocks.get().is_empty(),
            "no backend pads enumerated in tests"
        );
        assert!(
            !presentation
                .shortcuts_page
                .get()
                .first()
                .expect("shortcuts page builds")
                .model
                .snapshot()
                .rows
                .get()
                .is_empty(),
            "shortcuts page exposes rows"
        );
        assert_eq!(
            presentation.controllers_pages.get().len(),
            editor.vm.inputs().len(),
            "one controllers page per machine"
        );
        let general = editor.vm.general.view.get();
        assert_eq!(
            app.language
                .as_ref()
                .expect("language row builds")
                .model
                .snapshot()
                .choices
                .get()
                .len(),
            general.language_choices.len()
        );
        let position = choice_position(&general.language_choices, &general.language)
            .map(|index| index.to_string())
            .unwrap_or_default();
        assert_eq!(
            app.language
                .as_ref()
                .expect("language row builds")
                .model
                .snapshot()
                .selected
                .get(),
            position
        );
        assert!(audio.volume.set(42.0));
        assert_eq!(editor.vm.audio.view.get().volume_percent, 42);
        let en = I18n::new(AppLanguage::English);
        assert_eq!(
            audio.volume_label.get(),
            format!(
                "{}: {:.0}%",
                en.text(UiText::MasterVolume),
                audio.volume.get()
            )
        );
        assert_eq!(
            audio.latency_label.get(),
            format!(
                "{}: {:.0} ms",
                en.text(UiText::Latency),
                audio.latency.get()
            )
        );
        assert_eq!(
            presentation.labels.title.get(),
            format!("nerust {}", en.text(UiText::Settings))
        );
        // Subscribing observes the live property; dropping ends the watch.
        let seen = Rc::new(RefCell::new(Vec::new()));
        let subscription = audio.volume.subscribe(Rc::new({
            let seen = seen.clone();
            move |value| seen.borrow_mut().push(value)
        }));
        assert!(!seen.borrow().is_empty());
        drop(subscription);
        assert!(!audio.volume.set(150.0));
        assert_eq!(editor.vm.audio.view.get().volume_percent, 42);
        assert!(audio.latency.set(50.0));
        assert_eq!(editor.vm.audio.view.get().latency_ms, 50);
        assert!(!audio.latency.set(500.0));
        assert_eq!(editor.vm.audio.view.get().latency_ms, 50);
        // Directory writes map empty text to the default location.
        let dir =
            std::env::temp_dir().join(format!("nerust-settings-directory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directory temp root");
        assert!(app.directory.set(dir.to_string_lossy().into_owned()));
        assert_eq!(
            editor.vm.general.view.get().storage_directory,
            dir.to_string_lossy()
        );
        assert!(app.directory.set(String::new()));
        assert!(editor.vm.general.view.get().storage_directory.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let japanese = choice_position(&general.language_choices, &AppLanguage::Japanese)
            .map(|index| index.to_string())
            .expect("japanese is a language choice");
        assert!(
            app.language
                .as_ref()
                .expect("language row builds")
                .model
                .snapshot()
                .selected
                .set(japanese)
        );
        assert_eq!(editor.vm.general.view.get().language, AppLanguage::Japanese);
        let ja = I18n::new(AppLanguage::Japanese);
        assert_eq!(presentation.labels.cancel.get(), ja.text(UiText::Cancel));
        assert!(
            !app.language
                .as_ref()
                .expect("language row builds")
                .model
                .snapshot()
                .selected
                .set("999".into())
        );
        assert!(
            !app.language
                .as_ref()
                .expect("language row builds")
                .model
                .snapshot()
                .selected
                .set("bogus".into())
        );
        assert!(presentation.info_duration.set("3000".into()));
        assert_eq!(editor.vm.notifications.view.get().info_duration_ms, 3000);
        assert!(!presentation.info_duration.set("soon".into()));
        assert_eq!(editor.vm.notifications.view.get().info_duration_ms, 3000);
        assert!(presentation.page.set("controllers".into()));
        assert_eq!(editor.page.with(|page| page.clone()), "controllers");
    }

    /// The YAML command shims hand back the footer commands, so the
    /// surface key handler and the buttons share one wiring.
    #[test]
    fn presentation_command_wiring_matches_footer_commands() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        let (adapter, _dir) = loaded_adapter();
        let editor = Editor::new(adapter.settings().clone(), &adapter, Rc::new(TestExecutor))
            .expect("editor builds on real factories");
        let view_slot = Rc::new(RefCell::new(None));
        let apply: Apply = Rc::new(|_: SettingsSnapshot| Ok(()));
        let quit: Rc<dyn Fn()> = Rc::new(|| {});
        let presentation = editor.presentation(editor.commands(&apply, quit, &view_slot));
        let argument = Binding::constant(String::new());
        let cancel = presentation.command_cancel(argument.clone());
        let accept = presentation.command_accept(argument.clone());
        let discard = presentation.command_discard(argument.clone());
        let keep = presentation.command_keep(argument);
        assert!(cancel.enabled.get());
        assert!(accept.enabled.get());
        assert!(!discard.enabled.get());
        assert!(!keep.enabled.get());
        let _ = cancel.invoke();
        assert!(editor.exit.get() == ExitChoice::DiscardApproved);
    }

    /// Choice and integer helpers: positions map both ways, unknown ids
    /// reject, and only finite in-range integers convert.
    #[test]
    fn choice_and_integer_helpers_cover_edges() {
        let executor: Rc<dyn panechord::Executor> = Rc::new(TestExecutor);
        let backing = Store::new(2u8, executor.clone());
        let selected = backing.bind(|value| *value, Some(|slot: &mut u8, value| *slot = value));
        let choices = Binding::constant(vec![
            ChoiceView {
                value: 7u8,
                label: "Seven".into(),
            },
            ChoiceView {
                value: 2u8,
                label: "Two".into(),
            },
        ]);
        let bound = choice(choices, selected);
        assert_eq!(bound.items.get().len(), 2);
        assert_eq!(bound.items.get()[0].label, "Seven");
        assert_eq!(bound.selected.get(), "1");
        assert!(bound.selected.set("0".into()));
        assert!(!bound.selected.set("9".into()));
        assert!(!bound.selected.set("nope".into()));
        assert_eq!(checked_integer(10.0, 10.0, 200.0), Ok(10));
        assert_eq!(checked_integer(200.0, 10.0, 200.0), Ok(200));
        assert!(checked_integer(9.0, 10.0, 200.0).is_err());
        assert!(checked_integer(10.5, 10.0, 200.0).is_err());
        let backing = Store::new(120u32, executor);
        let duration =
            integer_text(backing.bind(|value| *value, Some(|slot: &mut u32, value| *slot = value)));
        assert_eq!(duration.get(), "120");
        assert!(duration.set("45".into()));
        assert_eq!(duration.get(), "45");
        assert!(!duration.set("soon".into()));
        assert!(!duration.set(String::new()));
        assert_eq!(duration.get(), "45");
    }
}
