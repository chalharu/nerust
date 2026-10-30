use super::*;
use crate::{
    persistence::{self, Sandbox},
    smoke::until,
    wiring,
};
use futures::{
    channel::oneshot,
    executor::{LocalPool, LocalSpawner},
    task::LocalSpawnExt,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wakui2_runtime::{Binding, BlockingPool, Executor, Limits, Subscription};

mod perf;

struct Local(LocalSpawner);
impl Executor for Local {
    fn spawn(&self, future: futures::future::LocalBoxFuture<'static, ()>) {
        self.0.spawn_local(future).unwrap();
    }
}
struct Fixture {
    sandbox: Sandbox,
    pool: LocalPool,
    workers: Rc<RefCell<BlockingPool>>,
    model: Store<App>,
    _projection: Subscription,
    _saves: Subscription,
}
impl Fixture {
    fn new() -> Self {
        Self::with_limits(Limits::default(), false)
    }
    fn with_limits(limits: Limits, directory: bool) -> Self {
        let sandbox = Sandbox::new().unwrap();
        if directory {
            std::fs::create_dir(sandbox.path()).unwrap();
        }
        let pool = LocalPool::new();
        let model = Store::with_limits(
            App::new(Config::default()),
            Rc::new(Local(pool.spawner())),
            limits,
        );
        let workers = Rc::new(RefCell::new(BlockingPool::new(1, 4)));
        let projection = wiring::project(&model);
        let saves = wiring::saves(&model, workers.clone(), sandbox.path());
        Self {
            sandbox,
            pool,
            workers,
            model,
            _projection: projection,
            _saves: saves,
        }
    }
    fn edit(&self, name: &str) {
        let name = name.to_string();
        assert!(self.model.update(move |s| s.draft.name = name));
    }
    fn save(&self, intent: SaveIntent) -> bool {
        self.model.command_save(Binding::constant(intent)).invoke()
    }
    fn done(&mut self) {
        self.pool
            .run_until(until(self.model.bind(App::busy, None), |busy| !busy));
    }
    /// Occupy the sole worker deterministically. UI futures await the startup
    /// signal; only the worker blocks on release. No UI thread Barrier::wait.
    fn block_worker(&mut self) -> oneshot::Sender<()> {
        let (started, ready) = oneshot::channel();
        let (release, blocked) = oneshot::channel();
        let task = self
            .workers
            .borrow()
            .try_spawn(move || {
                let _ = started.send(());
                futures::executor::block_on(crate::smoke::deadline(blocked)).unwrap();
            })
            .unwrap();
        self.pool.run_until(async {
            crate::smoke::deadline(ready).await.unwrap();
        });
        self.pool
            .spawner()
            .spawn_local(async {
                task.await.unwrap();
            })
            .unwrap();
        release
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.model.close();
        self.workers.borrow_mut().shutdown();
    }
}

#[test]
fn draft_invalid_intermediates_are_exact_strings() {
    let mut state = App::new(Config::default());
    for text in ["", "-", "abc", "101", "256"] {
        state.draft.volume = text.into();
        assert!(state.draft.parse().is_err());
        assert_eq!(state.draft.volume, text);
        assert_eq!(state.saved, Config::default());
    }
    state.draft.volume = "42".into();
    for text in ["", "-", "abc", "NaN", "inf", "0.4", "3.1"] {
        state.draft.scale = text.into();
        assert!(state.draft.parse().is_err());
        assert_eq!(state.draft.scale, text);
    }
    state.draft.scale = "1.5".into();
    state.draft.name = " ".into();
    assert!(state.draft.parse().is_err());
    state.draft.name = "Ada".into();
    let config = state.draft.parse().unwrap();
    assert_eq!(
        (config.name.as_str(), config.volume, config.scale),
        ("Ada", 42, 1.5)
    );
}

#[test]
fn cancel_restores_saved_clears_errors_without_io() {
    let f = Fixture::new();
    f.model.update(|s| {
        s.draft.volume = "-".into();
        s.io_error = "old error".into();
    });
    App::request_exit(f.model.clone());
    assert!(
        f.model
            .command_cancel(Binding::constant(String::new()))
            .invoke()
    );
    let state = f.model.snapshot();
    assert_eq!(state.draft, Draft::from(&state.saved));
    assert_eq!(state.saved, Config::default());
    assert!(state.ui.error.is_empty());
    assert!(state.io_error.is_empty());
    assert_eq!(state.exit, Exit::Editing);
    assert!(!f.sandbox.path().exists());
}

#[test]
fn save_success_only_updates_saved_after_completion() {
    let mut f = Fixture::new();
    f.edit("Ada");
    assert!(f.save(SaveIntent::Stay));
    assert_eq!(f.model.snapshot().saved.name, "Player");
    assert!(f.model.snapshot().busy());
    assert!(!f.save(SaveIntent::Stay));
    f.done();
    let state = f.model.snapshot();
    assert_eq!(state.saved.name, "Ada");
    assert!(!state.dirty());
    assert_eq!(persistence::load(&f.sandbox.path()).unwrap(), state.saved);
}

#[test]
fn failed_save_directory_preserves_state_and_recovers() {
    let mut f = Fixture::with_limits(Limits::default(), true);
    f.edit("Ada");
    let draft = f.model.snapshot().draft;
    assert!(f.save(SaveIntent::Exit));
    f.done();
    let state = f.model.snapshot();
    assert_eq!(state.saved, Config::default());
    assert_eq!(state.draft, draft);
    assert_eq!(state.exit, Exit::Confirm);
    assert!(state.io_error.contains("save"));
    assert_eq!(std::fs::read_dir(&f.sandbox.dir).unwrap().count(), 1);
    std::fs::remove_dir(f.sandbox.path()).unwrap();
    assert!(f.save(SaveIntent::Stay));
    f.done();
    assert_eq!(f.model.snapshot().saved.name, "Ada");
    assert!(f.model.snapshot().io_error.is_empty());
}

#[test]
fn editing_while_worker_blocked_keeps_new_draft_dirty() {
    let mut f = Fixture::new();
    let release = f.block_worker();
    f.edit("Captured");
    f.save(SaveIntent::Stay);
    f.pool.run_until_stalled(); // The save is submitted but cannot run yet.
    f.edit("Newer");
    assert_eq!(f.model.snapshot().saved.name, "Player");
    release.send(()).unwrap();
    f.done();
    let state = f.model.snapshot();
    assert_eq!(state.saved.name, "Captured");
    assert_eq!(state.draft.name, "Newer");
    assert!(state.dirty());
    assert_eq!(
        persistence::load(&f.sandbox.path()).unwrap().name,
        "Captured"
    );
}

#[test]
fn close_waits_for_save_and_reprompts_for_new_invalid_edit() {
    let mut f = Fixture::new();
    let release = f.block_worker();
    f.edit("Captured");
    f.save(SaveIntent::Exit);
    f.pool.run_until_stalled();
    f.model.update(|s| s.draft.volume = "-".into());
    App::request_exit(f.model.clone());
    assert_eq!(f.model.snapshot().exit, Exit::Waiting);
    assert!(
        !f.model
            .command_discard_exit(Binding::constant(String::new()))
            .invoke()
    );
    release.send(()).unwrap();
    f.done();
    let state = f.model.snapshot();
    assert_eq!(state.exit, Exit::Confirm);
    assert_eq!(state.draft.volume, "-");
    assert!(!state.close_ready());
    assert!(!state.can_save());
}

#[test]
fn close_reprompts_for_new_valid_edit_too() {
    let mut f = Fixture::new();
    f.edit("Captured");
    f.save(SaveIntent::Exit);
    f.edit("New valid edit");
    f.done();
    assert_eq!(f.model.snapshot().exit, Exit::Confirm);
    assert_eq!(f.model.snapshot().draft.name, "New valid edit");
}

#[test]
fn close_clean_cancel_discard_and_successful_save() {
    let mut f = Fixture::new();
    App::request_exit(f.model.clone());
    assert!(f.model.snapshot().close_ready());
    // New independent state for each terminal path.
    f.model.update(|s| *s = App::new(Config::default()));
    f.edit("Ada");
    App::request_exit(f.model.clone());
    assert_eq!(f.model.snapshot().exit, Exit::Confirm);
    assert!(
        f.model
            .command_keep_editing(Binding::constant(String::new()))
            .invoke()
    );
    assert_eq!(f.model.snapshot().draft.name, "Ada");
    App::request_exit(f.model.clone());
    assert!(
        f.model
            .command_discard_exit(Binding::constant(String::new()))
            .invoke()
    );
    assert!(f.model.snapshot().close_ready());
    assert_eq!(f.model.snapshot().saved, Config::default());
    assert!(!f.sandbox.path().exists());
    f.model.update(|s| *s = App::new(Config::default()));
    f.edit("Grace");
    assert!(
        !f.model
            .command_save_exit(Binding::constant(SaveIntent::Exit))
            .invoke()
    );
    App::request_exit(f.model.clone());
    assert!(
        f.model
            .command_save_exit(Binding::constant(SaveIntent::Exit))
            .invoke()
    );
    f.done();
    assert!(f.model.snapshot().close_ready());
    assert_eq!(f.model.snapshot().saved.name, "Grace");
}

#[test]
fn keep_editing_cancels_exit_while_save_continues() {
    let mut f = Fixture::new();
    f.edit("Ada");
    f.save(SaveIntent::Stay);
    App::request_exit(f.model.clone());
    assert_eq!(f.model.snapshot().exit, Exit::Waiting);
    assert!(
        f.model
            .command_keep_editing(Binding::constant(String::new()))
            .invoke()
    );
    f.done();
    assert_eq!(f.model.snapshot().exit, Exit::Editing);
    assert!(!f.model.snapshot().close_ready());
}

#[test]
fn busy_close_without_new_edits_closes_after_io_success() {
    let mut f = Fixture::new();
    f.edit("Ada");
    f.save(SaveIntent::Stay);
    App::request_exit(f.model.clone());
    assert!(!f.model.snapshot().close_ready());
    f.done();
    assert!(f.model.snapshot().close_ready());
    assert_eq!(persistence::load(&f.sandbox.path()).unwrap().name, "Ada");
}

#[test]
fn rejected_effect_never_submits_orphan_write() {
    let mut f = Fixture::with_limits(
        Limits {
            ingress: 256,
            tasks: 0,
        },
        false,
    );
    f.edit("Ada");
    assert!(f.save(SaveIntent::Stay)); // command accepted, effect rejected
    f.pool.run_until_stalled();
    assert!(!f.model.snapshot().busy());
    assert!(f.model.snapshot().io_error.contains("not accepted"));
    assert_eq!(f.model.snapshot().saved, Config::default());
    f.workers.borrow_mut().shutdown();
    assert!(!f.sandbox.path().exists());
}

#[test]
fn single_ingress_slot_saves_and_updates_projection() {
    let mut f = Fixture::with_limits(
        Limits {
            ingress: 1,
            tasks: 4,
        },
        false,
    );
    f.edit("容量一の保存");
    assert!(f.model.snapshot().ui.menu_items[0].enabled);
    assert!(f.save(SaveIntent::Stay));
    f.done();
    let state = f.model.snapshot();
    assert!(!state.busy());
    assert!(state.io_error.is_empty());
    assert!(!state.dirty());
    assert_eq!(state.saved.name, "容量一の保存");
    assert_eq!(persistence::load(&f.sandbox.path()).unwrap(), state.saved);
    assert!(!state.ui.menu_items[0].enabled);
}

#[test]
fn single_ingress_slot_task_rejection_recovers_and_preserves_draft() {
    let mut f = Fixture::with_limits(
        Limits {
            ingress: 1,
            tasks: 0,
        },
        false,
    );
    f.edit("未保存の下書き");
    assert!(f.save(SaveIntent::Exit));
    f.pool.run_until_stalled();
    let state = f.model.snapshot();
    assert!(!state.busy());
    assert_eq!(state.exit, Exit::Confirm);
    assert_eq!(state.draft.name, "未保存の下書き");
    assert_eq!(state.saved, Config::default());
    assert!(state.io_error.contains("not accepted"));
    assert!(state.ui.status.contains("not accepted"));
    f.workers.borrow_mut().shutdown();
    assert!(!f.sandbox.path().exists());
}

#[test]
fn closed_store_before_first_poll_never_submits_worker() {
    let mut f = Fixture::new();
    f.edit("Ada");
    f.save(SaveIntent::Stay);
    f.model.close();
    f.pool.run_until_stalled();
    f.workers.borrow_mut().shutdown();
    assert!(!f.sandbox.path().exists());
}

#[test]
fn pool_shutdown_is_recoverable_error_not_saved_success() {
    let mut f = Fixture::new();
    f.workers.borrow_mut().shutdown();
    f.edit("Ada");
    f.save(SaveIntent::Exit);
    f.done();
    assert_eq!(f.model.snapshot().exit, Exit::Confirm);
    assert_eq!(f.model.snapshot().saved, Config::default());
    assert!(f.model.snapshot().io_error.contains("shut down"));
}

#[test]
fn projection_has_no_feedback_and_menu_tracks_validation() {
    let f = Fixture::new();
    let notifications = Rc::new(Cell::new(0));
    let count = notifications.clone();
    let token = f
        .model
        .bind(|s| s.ui.clone(), None)
        .subscribe(Rc::new(move |_| count.set(count.get() + 1)));
    f.edit("Ada");
    let after_edit = notifications.get();
    assert!(f.model.snapshot().ui.menu_items[0].enabled);
    for _ in 0..10 {
        f.model.update(|_| {});
    }
    assert_eq!(notifications.get(), after_edit);
    f.model.update(|s| s.draft.volume = "-".into());
    assert!(!f.model.snapshot().ui.menu_items[0].enabled);
    assert_eq!(
        f.model.snapshot().ui.menu_items[0].shortcut.as_deref(),
        Some("Primary+S")
    );
    assert_eq!(
        f.model.snapshot().ui.menu_items[1].shortcut.as_deref(),
        Some("Primary+Q")
    );
    assert_eq!(f.model.snapshot().ui.menu_items[1].role, MenuRole::Quit);
    drop(token);
}

#[test]
fn typed_save_and_noarg_string_commands_remain_compatible() {
    let mut f = Fixture::new();
    f.edit("Ada");
    assert!(
        f.model
            .command_choose(Binding::constant("save".to_string()))
            .invoke()
    );
    f.done();
    assert_eq!(f.model.snapshot().saved.name, "Ada");
    f.edit("Later");
    assert!(
        f.model
            .command_cancel(Binding::constant("ignored noarg string".into()))
            .invoke()
    );
    assert_eq!(f.model.snapshot().draft.name, "Ada");
}

#[test]
fn full_pool_is_an_error_and_never_changes_saved() {
    let mut f = Fixture::new();
    let release = f.block_worker();
    let queued: Vec<_> = (0..4)
        .map(|_| f.workers.borrow().try_spawn(|| ()).unwrap())
        .collect();
    f.edit("Ada");
    assert!(f.save(SaveIntent::Stay));
    f.done();
    let state = f.model.snapshot();
    assert!(state.io_error.contains("queue is full"));
    assert_eq!(state.saved, Config::default());
    assert_eq!(state.draft.name, "Ada");
    assert!(!f.sandbox.path().exists());
    drop(queued);
    release.send(()).unwrap();
}

#[test]
fn numeric_formatting_is_only_normalized_for_unchanged_draft() {
    let mut f = Fixture::new();
    f.model.update(|s| s.draft.volume = "070".into());
    assert!(f.save(SaveIntent::Stay));
    f.model.update(|s| s.draft.volume = "0070".into());
    f.done();
    assert_eq!(f.model.snapshot().saved.volume, 70);
    assert_eq!(f.model.snapshot().draft.volume, "0070");
    assert!(f.model.snapshot().dirty());
    assert!(f.save(SaveIntent::Stay));
    f.done();
    assert_eq!(f.model.snapshot().draft.volume, "70");
    assert!(!f.model.snapshot().dirty());
}
