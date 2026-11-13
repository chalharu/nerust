mod layout;

use layout::BindingRowCommands;
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
    i18n::{I18n, UiText},
    model::language::AppLanguage,
};
use panechord::backend::{View, WeakView};
use panechord::{Binding, Choice, Command, Executor, Store, Subscription};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

type Setter<T> = Rc<dyn Fn(T) -> Result<(), ViewModelError>>;
pub(crate) type Apply = Rc<dyn Fn(SettingsSnapshot) -> Result<(), String>>;

/// Validation issues rendered in the draft language at the call site.
/// The view model only carries structured kinds; text resolves here.
fn issues_text(language: AppLanguage, issues: &[ValidationIssue]) -> String {
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
    closed: Rc<Cell<bool>>,
    status: Store<String>,
    page: Store<String>,
    input_page: Store<String>,
    system_page: Store<String>,
    exit: Rc<Cell<ExitChoice>>,
    close_prompt: Store<ExitChoice>,
}

impl Editor {
    pub(crate) fn new(
        snapshot: SettingsSnapshot,
        adapter: &crate::SessionAdapter,
        executor: Rc<dyn Executor>,
    ) -> Result<Rc<Self>, String> {
        let vm = SettingsViewModel::new(
            snapshot.clone(),
            adapter.factories().to_vec(),
            adapter.supported_sample_rates(),
            Rc::new(FilesystemValidator),
        )
        .map_err(|error| error.to_string())?;
        Ok(Rc::new(Self {
            vm,
            baseline: Rc::new(RefCell::new(snapshot)),
            closed: Rc::new(Cell::new(false)),
            status: Store::new(String::new(), executor.clone()),
            page: Store::new("general".into(), executor.clone()),
            input_page: Store::new(String::new(), executor.clone()),
            system_page: Store::new(String::new(), executor.clone()),
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
    pub(crate) fn close(&self) {
        self.closed.set(true);
        self.vm.capture.cancel_capture();
        self.status.close();
        self.page.close();
        self.input_page.close();
        self.system_page.close();
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
    fn tr(&self, key: UiText) -> Binding<String> {
        let general = self.vm.general.clone();
        self.binding(
            general.view.clone(),
            move |value| I18n::resolve_os(value.language).text(key).to_string(),
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
        Commands {
            cancel,
            accept,
            discard,
            keep,
            capture,
            clear,
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
        let Commands {
            cancel,
            accept,
            discard,
            keep,
            capture,
            clear,
        } = commands;
        Presentation {
            input_pages: Binding::constant(layout::inputs(self, capture, clear)),
            system_pages: Binding::constant(layout::systems(self)),
            input_selection: self
                .input_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            system_selection: self
                .system_page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            page: self
                .page
                .bind(Clone::clone, Some(|page: &mut String, value| *page = value)),
            status: self.status.bind(Clone::clone, None),
            confirmation: self.close_prompt.bind(
                |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
                None,
            ),
            language: choices!(general, language, language_choices, set_language),
            storage: choices!(
                general,
                storage_policy,
                storage_policy_choices,
                set_storage_policy
            ),
            directory,
            directory_visible: self.binding(
                self.vm.general.view.clone(),
                |value| value.show_storage_directory,
                None,
            ),
            fullscreen: field!(video, fullscreen_default, set_fullscreen_default),
            vsync: field!(video, vsync, set_vsync),
            scaling: choices!(video, scaling, scaling_choices, set_scaling),
            muted: field!(audio, muted, set_mute),
            volume_label: language.clone().zip(&volume).map(|(lang, value)| {
                format!(
                    "{}: {value:.0}%",
                    I18n::resolve_os(lang).text(UiText::MasterVolume)
                )
            }),
            volume,
            latency_label: language.clone().zip(&latency).map(|(lang, value)| {
                format!(
                    "{}: {value:.0} ms",
                    I18n::resolve_os(lang).text(UiText::Latency)
                )
            }),
            latency,
            sample_rate: choices!(audio, sample_rate, sample_rate_choices, set_sample_rate),
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
                            I18n::resolve_os(value.language).text(UiText::Settings)
                        )
                    },
                    None,
                ),
                tab_general: self.tr(UiText::General),
                tab_input: self.tr(UiText::Input),
                tab_video: self.tr(UiText::Video),
                tab_audio: self.tr(UiText::Audio),
                tab_notifications: self.tr(UiText::Notifications),
                tab_system: self.tr(UiText::System),
                language: self.tr(UiText::Language),
                storage_policy: self.tr(UiText::SaveStoragePolicy),
                fullscreen: self.tr(UiText::FullscreenDefault),
                vsync: self.tr(UiText::Vsync),
                scaling: self.tr(UiText::Scaling),
                mute: self.tr(UiText::Mute),
                sample_rate: self.tr(UiText::SampleRate),
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

/// Footer and input-row commands assembled by `Editor::commands`.
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
}

/// Static YAML captions as language-following bindings. Literals never
/// update; every settings caption rides the draft language through these
/// so switching UI language re-renders the open dialog in place.
#[derive(Clone)]
struct Labels {
    title: Binding<String>,
    tab_general: Binding<String>,
    tab_input: Binding<String>,
    tab_video: Binding<String>,
    tab_audio: Binding<String>,
    tab_notifications: Binding<String>,
    tab_system: Binding<String>,
    language: Binding<String>,
    storage_policy: Binding<String>,
    fullscreen: Binding<String>,
    vsync: Binding<String>,
    scaling: Binding<String>,
    mute: Binding<String>,
    sample_rate: Binding<String>,
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
    input_pages: Binding<Vec<panechord::Keyed<layout::InputPage>>>,
    system_pages: Binding<Vec<panechord::Keyed<layout::SystemPage>>>,
    input_selection: Binding<String>,
    system_selection: Binding<String>,
    page: Binding<String>,
    status: Binding<String>,
    confirmation: Binding<bool>,
    language: ChoiceBinding,
    storage: ChoiceBinding,
    directory: Binding<String>,
    directory_visible: Binding<bool>,
    fullscreen: Binding<bool>,
    vsync: Binding<bool>,
    scaling: ChoiceBinding,
    muted: Binding<bool>,
    volume: Binding<f64>,
    volume_label: Binding<String>,
    latency: Binding<f64>,
    latency_label: Binding<String>,
    sample_rate: ChoiceBinding,
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
        assert!(presentation.muted.set(true));
        assert!(presentation.status.get().contains("Master volume"));
        editor.close();
        assert!(!writer.set(None));
    }

    /// Presentation bindings mirror the draft and round-trip writes:
    /// numbers convert, choices reposition, durations parse as text.
    #[test]
    fn presentation_bindings_mirror_draft_and_round_trip_writes() {
        use crate::session::test_fixture::loaded_adapter;
        use nerust_gui_application::SettingsPort;
        use nerust_settings_core::model::language::AppLanguage;
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
        assert_eq!(
            presentation.fullscreen.get(),
            editor.vm.video.view.get().fullscreen_default
        );
        assert_eq!(presentation.vsync.get(), editor.vm.video.view.get().vsync);
        assert_eq!(presentation.muted.get(), editor.vm.audio.view.get().muted);
        assert_eq!(
            presentation.volume.get(),
            f64::from(editor.vm.audio.view.get().volume_percent)
        );
        assert_eq!(
            presentation.latency.get(),
            f64::from(editor.vm.audio.view.get().latency_ms)
        );
        assert_eq!(
            presentation.directory.get(),
            editor.vm.general.view.get().storage_directory
        );
        assert_eq!(
            presentation.directory_visible.get(),
            editor.vm.general.view.get().show_storage_directory
        );
        assert_eq!(
            presentation.input_pages.get().len(),
            editor.vm.inputs().len() + 1
        );
        assert_eq!(
            presentation.system_pages.get().len(),
            editor.vm.systems().len()
        );
        let general = editor.vm.general.view.get();
        assert_eq!(
            presentation.language.items.get().len(),
            general.language_choices.len()
        );
        let position = choice_position(&general.language_choices, &general.language)
            .map(|index| index.to_string())
            .unwrap_or_default();
        assert_eq!(presentation.language.selected.get(), position);
        assert!(presentation.volume.set(42.0));
        assert_eq!(editor.vm.audio.view.get().volume_percent, 42);
        assert_eq!(
            presentation.volume_label.get(),
            format!(
                "{}: {:.0}%",
                I18n::new(AppLanguage::English).text(UiText::MasterVolume),
                presentation.volume.get()
            )
        );
        assert_eq!(
            presentation.latency_label.get(),
            format!(
                "{}: {:.0} ms",
                I18n::new(AppLanguage::English).text(UiText::Latency),
                presentation.latency.get()
            )
        );
        assert_eq!(
            presentation.labels.title.get(),
            format!(
                "nerust {}",
                I18n::new(AppLanguage::English).text(UiText::Settings)
            )
        );
        // Subscribing observes the live property; dropping ends the watch.
        let seen = Rc::new(RefCell::new(Vec::new()));
        let subscription = presentation.volume.subscribe(Rc::new({
            let seen = seen.clone();
            move |value| seen.borrow_mut().push(value)
        }));
        assert!(!seen.borrow().is_empty());
        drop(subscription);
        assert!(!presentation.volume.set(150.0));
        assert_eq!(editor.vm.audio.view.get().volume_percent, 42);
        assert!(presentation.latency.set(50.0));
        assert_eq!(editor.vm.audio.view.get().latency_ms, 50);
        assert!(!presentation.latency.set(500.0));
        assert_eq!(editor.vm.audio.view.get().latency_ms, 50);
        // Directory writes map empty text to the default location.
        let dir =
            std::env::temp_dir().join(format!("nerust-settings-directory-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directory temp root");
        assert!(
            presentation
                .directory
                .set(dir.to_string_lossy().into_owned())
        );
        assert_eq!(
            editor.vm.general.view.get().storage_directory,
            dir.to_string_lossy()
        );
        assert!(presentation.directory.set(String::new()));
        assert!(editor.vm.general.view.get().storage_directory.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let japanese = choice_position(&general.language_choices, &AppLanguage::Japanese)
            .map(|index| index.to_string())
            .expect("japanese is a language choice");
        assert!(presentation.language.selected.set(japanese));
        assert_eq!(editor.vm.general.view.get().language, AppLanguage::Japanese);
        assert_eq!(
            presentation.labels.cancel.get(),
            I18n::new(AppLanguage::Japanese)
                .text(UiText::Cancel)
                .to_string()
        );
        assert!(!presentation.language.selected.set("999".into()));
        assert!(!presentation.language.selected.set("bogus".into()));
        assert!(presentation.info_duration.set("3000".into()));
        assert_eq!(editor.vm.notifications.view.get().info_duration_ms, 3000);
        assert!(!presentation.info_duration.set("soon".into()));
        assert_eq!(editor.vm.notifications.view.get().info_duration_ms, 3000);
        assert!(presentation.page.set("audio".into()));
        assert_eq!(editor.page.with(|page| page.clone()), "audio");
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
