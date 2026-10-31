use nerust_gui_runtime::settings::{
    SettingsSnapshot,
    apply::{DirectoryValidationError, validate_directory_path_typed},
};
use nerust_gui_viewmodel::settings::{
    ReadOnlyObservableProperty, SettingsViewModel, StoragePathError, StoragePathValidator,
    ViewModelError,
    dto::{ChoiceView, choice_position},
};
use panechord::{
    Binding, Choice, Executor, Layout, Parent, SliderRange, Store, Subscription, View,
};
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
    baseline: Rc<RefCell<SettingsSnapshot>>,
    closed: Rc<Cell<bool>>,
    status: Store<String>,
    page: Store<String>,
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
            exit: Rc::new(Cell::new(ExitChoice::Editing)),
            close_prompt: Store::new(ExitChoice::Editing, executor),
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
        let snapshot = self.vm.finish().map_err(|validation| {
            validation
                .issues
                .iter()
                .map(|issue| issue.message.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        let result = apply(snapshot.clone());
        if result.is_ok() {
            self.baseline.replace(snapshot);
        }
        let message = result
            .as_ref()
            .map_or_else(Clone::clone, |_| "Up to date".into());
        let _ = self.status.update(move |status| *status = message);
        result
    }

    pub(crate) fn build(self: &Rc<Self>, apply: Apply, quit: Rc<dyn Fn()>) -> Result<View, String> {
        let view = View::new(Binding::constant("nerust Settings".into()));
        let content = view.layout(
            &view.root(),
            "content",
            Layout::Stack {
                horizontal: false,
                gap: 12.0,
            },
        );
        view.style(
            "content",
            panechord::Style {
                padding: panechord::Edges {
                    left: 12.0,
                    top: 12.0,
                    right: 12.0,
                    bottom: 12.0,
                },
                ..Default::default()
            },
        );
        let tabs = view.tab(
            &content,
            "pages",
            self.page.bind(
                |page| page.clone(),
                Some(|page: &mut String, value: String| *page = value),
            ),
        );
        let general = view.tab_page(
            &tabs,
            "general",
            Binding::constant("General".into()),
            "general",
        );
        let general = view.layout(
            &general,
            "general_fields",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        view.label(
            &general,
            "language_label",
            Binding::constant("Language".into()),
        );
        let vm = self.vm.general.clone();
        combo(
            &view,
            &general,
            "language",
            self.binding(
                vm.view.clone(),
                |value| value.language_choices.clone(),
                None,
            ),
            self.binding(
                vm.view.clone(),
                |value| value.language,
                Some(Rc::new(move |value| vm.set_language(value))),
            ),
        );
        view.label(
            &general,
            "storage_label",
            Binding::constant("Storage".into()),
        );
        let vm = self.vm.general.clone();
        combo(
            &view,
            &general,
            "storage",
            self.binding(
                vm.view.clone(),
                |value| value.storage_policy_choices.clone(),
                None,
            ),
            self.binding(
                vm.view.clone(),
                |value| value.storage_policy,
                Some(Rc::new(move |value| vm.set_storage_policy(value))),
            ),
        );
        let vm = self.vm.general.clone();
        view.entry(
            &general,
            "storage_directory",
            self.binding(
                vm.view.clone(),
                |value| value.storage_directory.clone(),
                Some(Rc::new(move |value| {
                    vm.set_storage_directory(if value.is_empty() {
                        None
                    } else {
                        Some(value.into())
                    })
                })),
            ),
        );
        view.visible(
            "storage_directory",
            self.binding(
                self.vm.general.view.clone(),
                |value| value.show_storage_directory,
                None,
            ),
        );

        let video = view.tab_page(&tabs, "video", Binding::constant("Video".into()), "video");
        let video = view.layout(
            &video,
            "video_fields",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        let vm = self.vm.video.clone();
        view.checkbox(
            &video,
            "fullscreen",
            Binding::constant("Start fullscreen".into()),
            self.binding(
                vm.view.clone(),
                |value| value.fullscreen_default,
                Some(Rc::new(move |value| vm.set_fullscreen_default(value))),
            ),
        );
        let vm = self.vm.video.clone();
        view.checkbox(
            &video,
            "vsync",
            Binding::constant("VSync".into()),
            self.binding(
                vm.view.clone(),
                |value| value.vsync,
                Some(Rc::new(move |value| vm.set_vsync(value))),
            ),
        );
        view.label(&video, "scaling_label", Binding::constant("Scaling".into()));
        let vm = self.vm.video.clone();
        combo(
            &view,
            &video,
            "scaling",
            self.binding(vm.view.clone(), |value| value.scaling_choices.clone(), None),
            self.binding(
                vm.view.clone(),
                |value| value.scaling,
                Some(Rc::new(move |value| vm.set_scaling(value))),
            ),
        );

        let audio = view.tab_page(&tabs, "audio", Binding::constant("Audio".into()), "audio");
        let audio = view.layout(
            &audio,
            "audio_fields",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        let vm = self.vm.audio.clone();
        view.checkbox(
            &audio,
            "mute",
            Binding::constant("Mute".into()),
            self.binding(
                vm.view.clone(),
                |value| value.muted,
                Some(Rc::new(move |value| vm.set_mute(value))),
            ),
        );
        let vm = self.vm.audio.clone();
        let volume = self
            .binding(
                vm.view.clone(),
                |value| value.volume_percent,
                Some(Rc::new(move |value| vm.set_volume(value))),
            )
            .convert(f64::from, |value| Ok(value.round() as u8));
        view.label(
            &audio,
            "volume_label",
            volume.map(|value| format!("Volume: {value:.0}%")),
        );
        view.slider(
            &audio,
            "volume",
            volume,
            SliderRange {
                min: 0.0,
                max: 100.0,
                step: 1.0,
            },
        );
        view.label(
            &audio,
            "rate_label",
            Binding::constant("Sample rate".into()),
        );
        let vm = self.vm.audio.clone();
        combo(
            &view,
            &audio,
            "sample_rate",
            self.binding(
                vm.view.clone(),
                |value| value.sample_rate_choices.clone(),
                None,
            ),
            self.binding(
                vm.view.clone(),
                |value| value.sample_rate,
                Some(Rc::new(move |value| vm.set_sample_rate(value))),
            ),
        );
        let vm = self.vm.audio.clone();
        let latency = self
            .binding(
                vm.view.clone(),
                |value| value.latency_ms,
                Some(Rc::new(move |value| vm.set_latency(value))),
            )
            .convert(f64::from, |value| Ok(value.round() as u16));
        view.label(
            &audio,
            "latency_label",
            latency.map(|value| format!("Latency: {value:.0} ms")),
        );
        view.slider(
            &audio,
            "latency",
            latency,
            SliderRange {
                min: 10.0,
                max: 200.0,
                step: 1.0,
            },
        );
        view.finish_tab(&tabs)?;
        view.label(&content, "status", self.status.bind(Clone::clone, None));
        let editor = self.clone();
        let accepted = self.clone();
        let command = self.status.command(
            move |_| !accepted.closed.get() && accepted.dirty() && accepted.vm.finish().is_ok(),
            move |_, ()| {
                let _ = editor.apply(&apply);
            },
            Binding::constant(()),
        );
        view.button(
            &content,
            "apply",
            Binding::constant("Apply".into()),
            command,
        );
        let confirmation = view.layout(
            &content,
            "confirmation",
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
        );
        view.visible(
            "confirmation",
            self.close_prompt.bind(
                |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
                None,
            ),
        );
        view.label(
            &confirmation,
            "confirm_label",
            Binding::constant("Discard unsaved changes?".into()),
        );
        let actions = view.layout(
            &confirmation,
            "confirm_actions",
            Layout::Stack {
                horizontal: true,
                gap: 8.0,
            },
        );
        let weak = view.downgrade();
        let exit = self.exit.clone();
        let discard = self.close_prompt.command(
            |prompt| matches!(prompt, ExitChoice::ConfirmLocal | ExitChoice::ConfirmGlobal),
            move |_, ()| {
                let global = exit.get() == ExitChoice::ConfirmGlobal;
                exit.set(ExitChoice::DiscardApproved);
                if global {
                    quit();
                } else if let Some(view) = weak.upgrade() {
                    view.close();
                }
            },
            Binding::constant(()),
        );
        view.button(
            &actions,
            "discard",
            Binding::constant("Discard".into()),
            discard,
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
        view.button(
            &actions,
            "keep",
            Binding::constant("Keep Editing".into()),
            keep,
        );
        Ok(view)
    }
}

fn combo<T: Clone + Eq + 'static>(
    view: &View,
    parent: &Parent,
    id: &str,
    choices: Binding<Vec<ChoiceView<T>>>,
    selected: Binding<T>,
) {
    let read = choices.clone();
    let write = choices.clone();
    let selected = selected.convert(
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
    );
    view.combo(
        parent,
        id,
        choices.map(|choices| {
            choices
                .into_iter()
                .enumerate()
                .map(|(index, choice)| Choice {
                    id: index.to_string(),
                    label: choice.label,
                })
                .collect()
        }),
        selected,
    );
}
