//! Single-owner debugger execution. Native adapters own scheduling and rendering only.

use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    task::Poll,
};

use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DebugDrainOutput, DrainFlight, LiveImagesFlight},
    session::{LiveImages, SessionHandle, access::PauseControl},
};
use nerust_gui_viewmodel::debugger::DebugImageRgba;

use super::{DebuggerEvent, DebuggerModel, DebuggerScope, DrainSnapshot};
use crate::{
    application::ApplicationHost,
    drain::{DrainBatch, DrainEvent, DrainQueue, Flight, StartedDrain, prepare_pause_actions},
};

/// Owned native effects. Apply these only after releasing the controller borrow.
#[derive(Debug, Default)]
pub struct DebuggerEffects {
    pub scope: Option<DebuggerScope>,
    pub open_ppu: bool,
    /// Clear native staged inputs; the gesture belonged to the replaced model.
    pub epoch_reset: bool,
    /// `poll` requests another scheduled poll. Dispatch only queues work; call
    /// `poll` after dispatch/enqueue even when this flag is false.
    pub pending: bool,
}

#[derive(Debug, Default)]
pub struct DebuggerLiveEffects {
    pub changed: bool,
    pub running: bool,
    pub pending: bool,
}

#[derive(Clone)]
struct Snapshot {
    scope: DebuggerScope,
    input: DrainSnapshot,
}

/// Toolkit-free controller for an owned debugger model, FIFO drains and live images.
/// Generic flights permit deterministic, headless protocol tests; native callers
/// use the default shell flights. No timers, widget handles or shadow model.
pub struct DebuggerDriver<F = DrainFlight, L = LiveImagesFlight> {
    model: DebuggerModel,
    drain: DrainQueue<Snapshot, F>,
    live: Option<StartedDrain<(), L>>,
}

impl<F, L> Deref for DebuggerDriver<F, L> {
    type Target = DebuggerModel;
    fn deref(&self) -> &Self::Target {
        &self.model
    }
}

impl<F, L> DerefMut for DebuggerDriver<F, L> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.model
    }
}

impl<F, L> DebuggerDriver<F, L> {
    pub fn new(model: DebuggerModel) -> Self {
        Self {
            model,
            drain: Default::default(),
            live: None,
        }
    }

    pub fn from_session(session: &SessionHandle) -> Self {
        Self::new(DebuggerModel::from_session(session))
    }

    /// Call before reading native gesture inputs. Replacement drops all old work,
    /// including live flights and persistent write targets, even for local intents.
    pub fn reset_replaced_epoch(&mut self, session: &SessionHandle) -> bool {
        let epoch = session.debug_epoch();
        if Arc::ptr_eq(&self.model.epoch, &epoch) {
            return false;
        }
        self.replace_model(DebuggerModel::from_session(session));
        true
    }

    fn replace_model(&mut self, model: DebuggerModel) {
        self.drain.clear();
        self.live = None;
        self.model = model;
    }

    pub fn pending(&self) -> bool {
        self.drain.pending()
    }
    pub fn live_pending(&self) -> bool {
        self.live.is_some()
    }

    /// Input echoes and their gesture are accepted together, never into a new
    /// epoch. Actions are queued here; the adapter only handles native effects.
    pub fn dispatch(
        &mut self,
        session: &SessionHandle,
        inputs: impl IntoIterator<Item = DebuggerEvent>,
        event: DebuggerEvent,
        need_images: bool,
    ) -> DebuggerEffects {
        if self.reset_replaced_epoch(session) {
            self.enqueue_current(&[DebugAction::Refresh], DebuggerScope::All, need_images);
            return DebuggerEffects {
                epoch_reset: true,
                ..Default::default()
            };
        }
        for input in inputs {
            self.model.dispatch(input);
        }
        let update = self.model.dispatch(event);
        let mut effects = DebuggerEffects {
            open_ppu: update.open_ppu,
            ..Default::default()
        };
        if let Some(scope) = update.scope {
            if update.actions.is_empty() {
                effects.scope = Some(scope);
            } else {
                self.enqueue_current(&update.actions, scope, need_images || update.open_ppu);
            }
        }
        effects
    }

    /// Raw action port for initial Pause+Refresh. Old actions become refresh only.
    pub fn enqueue(
        &mut self,
        session: &SessionHandle,
        actions: &[DebugAction],
        scope: DebuggerScope,
        need_images: bool,
    ) {
        if self.reset_replaced_epoch(session) {
            self.enqueue_current(&[DebugAction::Refresh], DebuggerScope::All, need_images);
        } else {
            self.enqueue_current(actions, scope, need_images);
        }
    }

    fn enqueue_current(
        &mut self,
        actions: &[DebugAction],
        scope: DebuggerScope,
        need_images: bool,
    ) {
        let input = DrainSnapshot {
            space: self.model.selected_space(),
            mem_addr: self.model.nav.mem_addr(),
            dis_addr: self.model.nav.dis_addr(),
            watch: self.model.watch,
            freeze: self.model.freeze,
            reg_target: self.model.reg_target,
            prev_rows: self.model.display.dump_rows.clone().into(),
            ppu_open: need_images,
        };
        self.drain
            .enqueue(self.model.epoch.clone(), actions, Snapshot { scope, input });
    }

    /// Image-free rendering capture: native rebuilds may reenter the controller.
    pub fn view_snapshot(&self) -> DebuggerModel {
        let mut model = DebuggerModel::new(
            self.model.display.clone_without_images(),
            self.model.spaces.clone(),
            self.model.nav.mem_addr(),
            self.model.language,
            self.model.epoch.clone(),
        );
        *model = (*self.model).clone();
        model
    }
}

impl<F: Flight<Output = Box<DebugDrainOutput>>, L> DebuggerDriver<F, L> {
    fn poll_with(
        &mut self,
        mut begin: impl FnMut(&mut [DebugAction], &DrainSnapshot) -> F,
    ) -> DebuggerEffects {
        let mut effects = DebuggerEffects::default();
        loop {
            match self.drain.poll() {
                DrainEvent::Idle => return effects,
                DrainEvent::Pending => {
                    effects.pending = true;
                    return effects;
                }
                DrainEvent::Start(batch) => {
                    if let Some(flight) =
                        batch.begin(|actions, snapshot| begin(actions, &snapshot.input))
                    {
                        self.drain.store(flight);
                    }
                }
                DrainEvent::Done(output, snapshot) => {
                    self.model.apply_drain(output.display, output.reselected);
                    effects.scope = Some(union(effects.scope, snapshot.scope));
                }
            }
        }
    }
}

impl<F, L: Flight<Output = LiveImages>> DebuggerDriver<F, L> {
    fn poll_live_with(&mut self, running: bool) -> DebuggerLiveEffects {
        let mut effects = DebuggerLiveEffects {
            running,
            ..Default::default()
        };
        let Some(mut flight) = self.live.take() else {
            return effects;
        };
        match flight.poll() {
            Poll::Pending => {
                self.live = Some(flight);
                effects.pending = true;
            }
            Poll::Ready(Some(Some(batch))) => {
                effects.changed = self.model.apply_live_images(
                    batch.generation,
                    batch
                        .images
                        .into_iter()
                        .map(|image| DebugImageRgba {
                            label: image.label,
                            width: image.width,
                            height: image.height,
                            rgba: image.rgba,
                        })
                        .collect(),
                    running,
                );
            }
            Poll::Ready(_) => {}
        }
        effects
    }

    fn begin_live(&mut self, begin: impl FnOnce() -> L) {
        self.live = DrainBatch {
            epoch: self.model.epoch.clone(),
            actions: vec![],
            snapshot: (),
        }
        .begin(|_, _| begin());
    }

    fn live_tick_with(
        &mut self,
        running: bool,
        interested: bool,
        begin: impl FnOnce() -> L,
    ) -> DebuggerLiveEffects {
        let mut effects = self.poll_live_with(running);
        if effects.pending || !interested || !running {
            return effects;
        }
        self.begin_live(begin);
        let next = self.poll_live_with(running);
        effects.changed |= next.changed;
        effects.pending = next.pending;
        effects
    }
}

impl DebuggerDriver {
    /// Pause ownership goes through ApplicationHost, never raw session side effects.
    /// FIFO boundaries ensure a later resume cannot run ahead of an earlier write.
    pub fn poll(&mut self, host: &mut ApplicationHost) -> DebuggerEffects {
        self.poll_with(|actions, snapshot| {
            prepare_pause_actions(actions, |action| {
                match action {
                    DebugAction::Pause => host.pause(),
                    DebugAction::Resume => host.resume(),
                    DebugAction::TogglePause => host.toggle_pause(),
                    _ => unreachable!("pause actions only"),
                }
                host.tick().loaded
            });
            host.session().debug_drain_begin(
                actions,
                &DebugDrainInput {
                    space: snapshot.space,
                    mem_addr: snapshot.mem_addr,
                    dis_addr: snapshot.dis_addr,
                    watch: snapshot.watch,
                    freeze: snapshot.freeze,
                    reg_target: snapshot.reg_target,
                    prev_dump_rows: &snapshot.prev_rows,
                    need_images: snapshot.ppu_open,
                },
            )
        })
    }

    pub fn poll_live(&mut self, session: &SessionHandle) -> DebuggerLiveEffects {
        let tick = session.tick_metrics();
        self.poll_live_with(tick.loaded && !tick.paused)
    }

    /// Native cadence requests a turn; interest/running and single-flight policy
    /// live here. A paused late reply can update images but never running chrome.
    pub fn live_tick(
        &mut self,
        session: &SessionHandle,
        visible: bool,
        need_images: bool,
    ) -> DebuggerLiveEffects {
        let tick = session.tick_metrics();
        self.live_tick_with(
            tick.loaded && !tick.paused,
            visible && need_images && Arc::ptr_eq(&self.model.epoch, &session.debug_epoch()),
            || session.debug_live_images_begin(true),
        )
    }
}

fn union(old: Option<DebuggerScope>, next: DebuggerScope) -> DebuggerScope {
    use DebuggerScope::*;
    match (old, next) {
        (None, scope) | (Some(Header), scope) => scope,
        (Some(scope), Header) => scope,
        (Some(a), b) if a == b => a,
        (Some(Dump), State | Edit) | (Some(State | Edit), Dump) => Dump,
        _ => All,
    }
}

#[cfg(test)]
mod tests;
