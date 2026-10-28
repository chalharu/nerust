//! Single-owner debugger execution. Native adapters own scheduling and rendering only.

use std::{sync::Arc, task::Poll};

use nerust_gui_shell::{
    debug::{DebugAction, DebugDrainInput, DebugDrainOutput, DrainFlight, LiveImagesFlight},
    session::{LiveImages, SessionHandle},
};
use nerust_gui_viewmodel::debugger::DebugImageRgba;
use wakui::debugger::{DebuggerEffects, DebuggerEvent, DebuggerLiveEffects, DebuggerScope};

use crate::{
    debugger::{BridgeView, DebugBridge, DrainSnapshot},
    drain::{DrainBatch, DrainEvent, DrainQueue, Flight, StartedDrain, prepare_pause_actions},
};

#[derive(Clone)]
struct Snapshot {
    scope: DebuggerScope,
    input: DrainSnapshot,
}

/// Toolkit-free drain orchestration over a shared bridge. The bridge owns
/// view state (possibly shared by several native windows); the driver owns
/// only queues, flights, and the bound epoch. Generic flights permit
/// deterministic, headless protocol tests; native callers use the default
/// shell flights. No timers or widget handles.
#[derive(Default)]
pub struct DebuggerDriver<F = DrainFlight, L = LiveImagesFlight> {
    epoch: Option<Arc<std::sync::atomic::AtomicBool>>,
    drain: DrainQueue<Snapshot, F>,
    live: Option<StartedDrain<(), L>>,
}

impl<F, L> DebuggerDriver<F, L> {
    pub fn new() -> Self {
        Self {
            epoch: None,
            drain: Default::default(),
            live: None,
        }
    }

    /// Call before reading native gesture inputs. Replacement drops all old work,
    /// including live flights and persistent write targets, even for local intents.
    /// The caller rebuilds the bridge when this reports a replacement.
    pub fn reset_replaced_epoch(&mut self, session: &SessionHandle) -> bool {
        let epoch = session.debug_epoch();
        if self
            .epoch
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &epoch))
        {
            return false;
        }
        self.drain.clear();
        self.live = None;
        self.epoch = Some(epoch);
        true
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
        bridge: &DebugBridge,
        inputs: impl IntoIterator<Item = DebuggerEvent>,
        event: DebuggerEvent,
        need_images: bool,
    ) -> DebuggerEffects {
        if self.reset_replaced_epoch(session) {
            self.enqueue_current(
                &session.debug_epoch(),
                bridge,
                &[DebugAction::Refresh],
                DebuggerScope::All,
                need_images,
            );
            return DebuggerEffects {
                epoch_reset: true,
                ..Default::default()
            };
        }
        for input in inputs {
            bridge.dispatch(input);
        }
        let update = bridge.dispatch(event);
        let mut effects = DebuggerEffects {
            open_ppu: update.open_ppu,
            ..Default::default()
        };
        if let Some(scope) = update.scope {
            if update.actions.is_empty() {
                effects.scope = Some(scope);
            } else {
                self.enqueue_current(
                    &session.debug_epoch(),
                    bridge,
                    &update.actions,
                    scope,
                    need_images || update.open_ppu,
                );
            }
        }
        effects
    }

    /// Raw action port for initial Pause+Refresh. Old actions become refresh only.
    pub fn enqueue(
        &mut self,
        session: &SessionHandle,
        bridge: &DebugBridge,
        actions: &[DebugAction],
        scope: DebuggerScope,
        need_images: bool,
    ) {
        if self.reset_replaced_epoch(session) {
            self.enqueue_current(
                &session.debug_epoch(),
                bridge,
                &[DebugAction::Refresh],
                DebuggerScope::All,
                need_images,
            );
        } else {
            self.enqueue_current(&session.debug_epoch(), bridge, actions, scope, need_images);
        }
    }

    /// Gesture-side refresh port: native adapters enqueue without naming actions.
    pub fn refresh(
        &mut self,
        session: &SessionHandle,
        bridge: &DebugBridge,
        scope: DebuggerScope,
        need_images: bool,
    ) {
        self.enqueue(session, bridge, &[DebugAction::Refresh], scope, need_images);
    }

    /// Gesture-side open port (Pause + Refresh) without naming actions.
    pub fn open_refresh(
        &mut self,
        session: &SessionHandle,
        bridge: &DebugBridge,
        need_images: bool,
    ) {
        self.enqueue(
            session,
            bridge,
            &[DebugAction::Pause, DebugAction::Refresh],
            DebuggerScope::All,
            need_images,
        );
    }

    fn enqueue_current(
        &mut self,
        epoch: &Arc<std::sync::atomic::AtomicBool>,
        bridge: &DebugBridge,
        actions: &[DebugAction],
        scope: DebuggerScope,
        need_images: bool,
    ) {
        let view: BridgeView = bridge.view_snapshot();
        let input = DrainSnapshot {
            space: bridge
                .spaces()
                .get(view.nav.space_idx())
                .map(|(id, _, _)| *id),
            mem_addr: view.nav.mem_addr(),
            dis_addr: view.nav.dis_addr(),
            watch: view.watch,
            freeze: view.freeze,
            reg_target: view.reg_target,
            prev_rows: bridge.display_snapshot().dump_rows.clone().into(),
            ppu_open: need_images,
        };
        self.drain
            .enqueue(epoch.clone(), actions, Snapshot { scope, input });
    }
}

impl<F: Flight<Output = Box<DebugDrainOutput>>, L> DebuggerDriver<F, L> {
    fn poll_with(
        &mut self,
        bridge: &DebugBridge,
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
                    bridge.apply_drain_display(output.display, output.reselected);
                    effects.scope = Some(union(effects.scope, snapshot.scope));
                }
            }
        }
    }
}

impl<F, L: Flight<Output = LiveImages>> DebuggerDriver<F, L> {
    fn poll_live_with(&mut self, bridge: &DebugBridge, running: bool) -> DebuggerLiveEffects {
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
                effects.changed = bridge.apply_live_images(
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

    fn begin_live(&mut self, epoch: Arc<std::sync::atomic::AtomicBool>, begin: impl FnOnce() -> L) {
        self.live = DrainBatch {
            epoch,
            actions: vec![],
            snapshot: (),
        }
        .begin(|_, _| begin());
    }

    fn live_tick_with(
        &mut self,
        bridge: &DebugBridge,
        epoch: Arc<std::sync::atomic::AtomicBool>,
        running: bool,
        interested: bool,
        begin: impl FnOnce() -> L,
    ) -> DebuggerLiveEffects {
        let mut effects = self.poll_live_with(bridge, running);
        if effects.pending || !interested || !running {
            return effects;
        }
        self.begin_live(epoch, begin);
        let next = self.poll_live_with(bridge, running);
        effects.changed |= next.changed;
        effects.pending = next.pending;
        effects
    }
}

impl DebuggerDriver {
    /// Pause ownership goes through the session's pause policy, never raw
    /// session side effects. FIFO boundaries ensure a later resume cannot
    /// run ahead of an earlier write.
    pub fn poll(
        &mut self,
        session: &mut nerust_gui_shell::session::SessionHandle,
        pause: &mut wakui::pause::PausePolicy,
        bridge: &DebugBridge,
    ) -> DebuggerEffects {
        use nerust_gui_shell::session::commands::SessionCommand;
        self.poll_with(bridge, |actions, snapshot| {
            prepare_pause_actions(actions, |action| {
                let command = match action {
                    DebugAction::Pause => SessionCommand::Pause,
                    DebugAction::Resume => SessionCommand::Resume,
                    DebugAction::TogglePause => SessionCommand::TogglePause,
                    _ => unreachable!("pause actions only"),
                };
                let _ = crate::session::dispatch(
                    session,
                    pause,
                    wakui::session::SessionEvent::Command(command),
                );
                session.tick_metrics().loaded
            });
            session.debug_drain_begin(
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

    pub fn poll_live(
        &mut self,
        session: &SessionHandle,
        bridge: &DebugBridge,
    ) -> DebuggerLiveEffects {
        let tick = session.tick_metrics();
        self.poll_live_with(bridge, tick.loaded && !tick.paused)
    }

    /// Native cadence requests a turn; interest/running and single-flight policy
    /// live here. A paused late reply can update images but never running chrome.
    pub fn live_tick(
        &mut self,
        session: &SessionHandle,
        bridge: &DebugBridge,
        visible: bool,
        need_images: bool,
    ) -> DebuggerLiveEffects {
        let tick = session.tick_metrics();
        let epoch_ok = self
            .epoch
            .as_ref()
            .is_some_and(|epoch| Arc::ptr_eq(epoch, &session.debug_epoch()));
        self.live_tick_with(
            bridge,
            session.debug_epoch(),
            tick.loaded && !tick.paused,
            visible && need_images && epoch_ok,
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
