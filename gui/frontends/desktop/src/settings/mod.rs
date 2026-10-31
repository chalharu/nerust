mod layout;

use layout::BindingRowCommands;
use nerust_gui_runtime::settings::{
    SettingsSnapshot,
    apply::{DirectoryValidationError, validate_directory_path_typed},
};
use nerust_gui_viewmodel::settings::{
    ReadOnlyObservableProperty, SettingsViewModel, StoragePathError, StoragePathValidator,
    ViewModelError,
    dto::{ChoiceView, choice_position},
};
use panechord::{Binding, Choice, Command, Executor, Store, Subscription, View, WeakView};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};

type Setter<T> = Rc<dyn Fn(T) -> Result<(), ViewModelError>>;
pub(crate) type Apply = Rc<dyn Fn(SettingsSnapshot) -> Result<(), String>>;

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
            status: Store::new("Up to date".into(), executor.clone()),
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
        self.status.close();
        self.page.close();
        self.input_page.close();
        self.system_page.close();
        self.close_prompt.close();
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
        let baseline = self.baseline.clone();
        let status = self.status.clone();
        let write = setter.map(|setter| {
            let writer: Rc<dyn Fn(T) -> Result<(), String>> = Rc::new(move |value| {
                if closed.get() {
                    return Err("settings editor is closed".into());
                }
                let result = setter(value).map_err(|error| error.to_string());
                let message = match &result {
                    Err(error) => error.clone(),
                    Ok(()) => match vm.finish() {
                        Err(validation) => validation
                            .issues
                            .iter()
                            .map(|issue| issue.message.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        Ok(snapshot) => if snapshot == *baseline.borrow() {
                            "Up to date"
                        } else {
                            "Unsaved changes"
                        }
                        .into(),
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
    fn apply(&self, apply: &Apply) -> Result<(), String> {
        let result = self
            .vm
            .finish()
            .map_err(|validation| {
                validation
                    .issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .and_then(|snapshot| {
                apply(snapshot.clone())?;
                self.baseline.replace(snapshot);
                Ok(())
            });
        let message = result
            .as_ref()
            .map_or_else(Clone::clone, |_| "Up to date".into());
        let _ = self.status.update(move |status| *status = message);
        result
    }
    pub(crate) fn build(self: &Rc<Self>, apply: Apply, quit: Rc<dyn Fn()>) -> Result<View, String> {
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
        let view_slot = Rc::new(RefCell::new(None::<WeakView>));
        let commit = apply.clone();
        let editor = self.clone();
        let accepted = self.clone();
        let apply = self.status.command(
            move |_| !accepted.closed.get() && accepted.dirty() && accepted.vm.finish().is_ok(),
            move |_, ()| {
                let _ = editor.apply(&apply);
            },
            Binding::constant(()),
        );
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
        let bindings = Presentation {
            input_pages: Binding::constant(layout::inputs(
                &self.vm,
                self.executor.clone(),
                capture,
            )),
            system_pages: Binding::constant(layout::systems(&self.vm, self.executor.clone())),
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
            volume_label: volume.map(|value| format!("Volume: {value:.0}%")),
            volume,
            latency_label: latency.map(|value| format!("Latency: {value:.0} ms")),
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
            apply,
            cancel,
            accept,
            discard,
            keep,
        };
        let view = panechord::try_view!("ui/settings.yaml", bindings, panechord)?;
        view_slot.replace(Some(view.downgrade()));
        Ok(view)
    }
}

#[derive(Clone)]
struct ChoiceBinding {
    items: Binding<Vec<Choice>>,
    selected: Binding<String>,
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
    apply: Command,
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
    fn command_apply(&self, _: Binding<String>) -> Command {
        self.apply.clone()
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
}
