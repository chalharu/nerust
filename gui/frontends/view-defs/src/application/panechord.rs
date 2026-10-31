use std::rc::Rc;

use ::panechord::{Binding, Executor, Store};
use nerust_gui_runtime::settings::SettingsSnapshot;

use super::ApplicationHost;

#[derive(Clone, PartialEq)]
pub struct HostSnapshot {
    pub loaded: bool,
    pub paused: bool,
    pub settings: Rc<SettingsSnapshot>,
}

pub struct PanechordState {
    state: Store<HostSnapshot>,
}

impl PanechordState {
    pub fn new(host: &ApplicationHost, executor: Rc<dyn Executor>) -> Self {
        let tick = host.tick();
        Self {
            state: Store::new(
                HostSnapshot {
                    loaded: tick.loaded,
                    paused: tick.paused,
                    settings: Rc::new(host.settings_snapshot().clone()),
                },
                executor,
            ),
        }
    }

    pub fn binding(&self) -> Binding<HostSnapshot> {
        self.state.bind(Clone::clone, None)
    }

    pub fn publish(&self, host: &ApplicationHost) -> bool {
        let current = self.state.snapshot();
        let tick = host.tick();
        let settings_changed = current.settings.as_ref() != host.settings_snapshot();
        let settings = if settings_changed {
            Rc::new(host.settings_snapshot().clone())
        } else {
            current.settings
        };
        self.state.update_scoped(&[], move |state| {
            let next = HostSnapshot {
                loaded: tick.loaded,
                paused: tick.paused,
                settings,
            };
            (state != &next).then_some(next)
        })
    }
}
