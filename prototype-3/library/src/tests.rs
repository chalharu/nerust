//! Display-free tests. Controlled scans use oneshots, never sleeps or polling loops.
use crate::{
    model::{App, AppCommands},
    scan::{self, Entry, Report},
    smoke::Sandbox,
    wiring,
};
use futures::{
    channel::oneshot,
    executor::{LocalPool, LocalSpawner},
    task::LocalSpawnExt,
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::PathBuf,
    rc::Rc,
};
use wakui2_runtime::{Binding, BlockingPool, Executor, Limits, Store, Subscription};

mod perf;

struct Local(LocalSpawner);
impl Executor for Local {
    fn spawn(&self, future: futures::future::LocalBoxFuture<'static, ()>) {
        self.0.spawn_local(future).unwrap();
    }
}
type Reply = oneshot::Sender<Result<Report, String>>;
struct Fixture {
    pool: LocalPool,
    model: Store<App>,
    replies: Rc<RefCell<VecDeque<Reply>>>,
    tokens: Vec<Subscription>,
}
impl Fixture {
    fn new(limits: Limits) -> Self {
        let pool = LocalPool::new();
        let executor = Rc::new(Local(pool.spawner()));
        let model = Store::with_limits(App::new("fixture".into()), executor.clone(), limits);
        let replies = Rc::new(RefCell::new(VecDeque::new()));
        let pending = replies.clone();
        let scan: wiring::Scanner = Rc::new(move |_, _| {
            let (send, receive) = oneshot::channel();
            pending.borrow_mut().push_back(send);
            Box::pin(async move { receive.await.map_err(|_| "scan canceled".to_string())? })
        });
        let tokens = wiring::connect(&model, executor, scan);
        Self {
            pool,
            model,
            replies,
            tokens,
        }
    }
    fn reply(&mut self, result: Result<Report, String>) {
        self.pool.run_until_stalled();
        assert!(
            self.replies
                .borrow_mut()
                .pop_back()
                .unwrap()
                .send(result)
                .is_ok()
        );
        self.pool.run_until_stalled();
    }
    fn rescan(&self) {
        assert!(
            self.model
                .command_rescan(Binding::constant(String::new()))
                .invoke()
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.model.close();
    }
}
fn entry(name: &str) -> Entry {
    Entry {
        path: PathBuf::from("fixture").join(name),
        name: name.into(),
        system: "NES".into(),
        bytes: 42,
    }
}
fn report(names: &[&str]) -> Report {
    Report {
        entries: names.iter().map(|name| entry(name)).collect(),
        capped: false,
    }
}

#[test]
fn query_during_scan_rejects_stale_result_and_keeps_edits_responsive() {
    let mut f = Fixture::new(Limits::default());
    f.pool.run_until_stalled();
    let stale = f.replies.borrow_mut().pop_front().unwrap();
    assert!(
        f.model
            .bind(|s| s.filter.clone(), Some(|s, value| s.filter = value))
            .set("new".into())
    );
    assert!(f.model.snapshot().busy);
    assert_eq!(f.model.snapshot().filter, "new");
    let _ = stale.send(Ok(report(&["old.nes"])));
    f.pool.run_until_stalled();
    assert!(f.model.snapshot().entries.is_empty());
    f.reply(Ok(report(&["new.nes"])));
    assert_eq!(f.model.snapshot().entries[0].name, "new.nes");
    assert!(!f.model.snapshot().busy);
}

#[test]
fn generation_and_query_both_guard_completions() {
    let mut state = App::new("fixture".into());
    state.begin(state.query());
    let old = state.pending.clone().unwrap();
    state.begin(state.query());
    state.complete(old, Ok(report(&["old.nes"])));
    assert!(state.entries.is_empty());
    let current = state.pending.clone().unwrap();
    state.filter = "new".into();
    state.complete(current, Ok(report(&["also-old.nes"])));
    assert!(state.busy);
    assert!(state.entries.is_empty());
}

#[test]
fn errors_recover_and_empty_result_clears_selection() {
    let mut f = Fixture::new(Limits::default());
    f.reply(Err("permission denied".into()));
    assert_eq!(f.model.snapshot().error, "permission denied");
    assert!(!f.model.snapshot().busy);
    f.rescan();
    f.reply(Ok(report(&["a.nes", "b.nes"])));
    assert!(f.model.snapshot().error.is_empty());
    assert!(
        f.model
            .command_next(Binding::constant(String::new()))
            .invoke()
    );
    assert!(f.model.snapshot().details.contains("b.nes"));
    f.rescan();
    f.reply(Ok(Report::default()));
    assert_eq!(f.model.snapshot().details, "No file selected");
    assert!(
        !f.model
            .command_next(Binding::constant(String::new()))
            .invoke()
    );
}

#[test]
fn ingress_one_runs_reactions_effects_and_recovery() {
    let mut f = Fixture::new(Limits {
        ingress: 1,
        tasks: 4,
    });
    f.reply(Ok(report(&["a.nes"])));
    assert_eq!(f.model.snapshot().rows.len(), 1);
    let _ = f.model.update(|s| s.filter = "b".into());
    f.reply(Err("retry me".into()));
    assert_eq!(f.model.snapshot().error, "retry me");
    f.rescan();
    f.reply(Ok(report(&["b.nes"])));
    assert_eq!(f.model.snapshot().rows.len(), 1);
    assert!(f.model.snapshot().error.is_empty());
}

#[test]
fn rejected_task_does_not_poll_or_submit_scanner() {
    let pool = LocalPool::new();
    let executor = Rc::new(Local(pool.spawner()));
    let model = Store::with_limits(
        App::new("unused".into()),
        executor.clone(),
        Limits {
            ingress: 1,
            tasks: 0,
        },
    );
    let polled = Rc::new(Cell::new(false));
    let observed = polled.clone();
    let scan: wiring::Scanner = Rc::new(move |_, _| {
        let observed = observed.clone();
        Box::pin(async move {
            observed.set(true);
            Ok(Report::default())
        })
    });
    let _tokens = wiring::connect(&model, executor, scan);
    assert!(!polled.get());
    assert!(!model.snapshot().busy);
    assert!(model.snapshot().error.contains("not accepted"));
}

#[test]
fn bounded_list_and_stable_keyed_models() {
    let mut f = Fixture::new(Limits::default());
    let entries = (0..scan::MAX_RESULTS + 20)
        .map(|i| entry(&format!("{i}.nes")))
        .collect();
    f.reply(Ok(Report {
        entries,
        capped: false,
    }));
    assert_eq!(f.model.snapshot().rows.len(), scan::MAX_RESULTS);
    assert!(f.model.snapshot().capped);
    let first = f.model.snapshot().rows[0].clone();
    f.rescan();
    f.reply(Ok(report(&["0.nes"])));
    assert!(first.model.ptr_eq(&f.model.snapshot().rows[0].model));
    assert!(wakui2_runtime::valid_item_key(&first.key));
}

#[test]
fn token_drop_stops_new_work_and_projection() {
    let mut f = Fixture::new(Limits::default());
    f.reply(Ok(report(&["a.nes"])));
    f.tokens.clear();
    let _ = f.model.update(|s| {
        s.filter = "changed".into();
        s.entries = std::sync::Arc::new(Vec::new());
    });
    f.pool.run_until_stalled();
    assert_eq!(f.model.snapshot().rows.len(), 1);
    assert!(f.replies.borrow().is_empty());
}

#[test]
fn watch_token_drop_invalidates_already_queued_reaction() {
    let pool = LocalPool::new();
    let model = Store::with_limits(
        0usize,
        Rc::new(Local(pool.spawner())),
        Limits {
            ingress: 1,
            tasks: 1,
        },
    );
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let token = Rc::new(RefCell::new(Some(
        model.watch(model.bind(|s| *s, None), move |_, _| {
            observed.set(observed.get() + 1)
        }),
    )));
    let before = calls.get();
    let remove = token.clone();
    let _dropper = model.bind(|s| *s, None).subscribe(Rc::new(move |value| {
        if value == 1 {
            remove.borrow_mut().take();
        }
    }));
    let _ = model.update(|s| *s = 1);
    assert_eq!(calls.get(), before);
}

#[test]
fn scan_actual_temp_metadata_is_nonwriting_and_case_insensitive() {
    let sandbox = Sandbox::new().unwrap();
    sandbox.populate().unwrap();
    std::fs::create_dir(sandbox.dir.join("directory.nes")).unwrap();
    let before = std::fs::read_dir(&sandbox.dir).unwrap().count();
    let original = std::fs::read(sandbox.dir.join("Alpha.nes")).unwrap();
    let report = scan::scan(sandbox.dir.clone(), String::new()).unwrap();
    assert_eq!(report.entries.len(), 4);
    assert!(
        report
            .entries
            .iter()
            .all(|e| e.bytes == original.len() as u64)
    );
    assert_eq!(
        scan::scan(sandbox.dir.clone(), "GAMMA".into())
            .unwrap()
            .entries[0]
            .system,
        "GBC"
    );
    assert_eq!(std::fs::read_dir(&sandbox.dir).unwrap().count(), before);
    assert_eq!(
        std::fs::read(sandbox.dir.join("Alpha.nes")).unwrap(),
        original
    );
    assert!(scan::scan(sandbox.dir.join("missing"), String::new()).is_err());
    assert!(scan::scan(sandbox.dir.join("Alpha.nes"), String::new()).is_err());
}

#[test]
fn actual_scan_caps_results_and_ignores_symlinks() {
    let sandbox = Sandbox::new().unwrap();
    for i in 0..scan::MAX_RESULTS + 1 {
        std::fs::write(sandbox.dir.join(format!("{i}.nes")), []).unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(sandbox.dir.join("0.nes"), sandbox.dir.join("link.gb")).unwrap();
    let report = scan::scan(sandbox.dir.clone(), String::new()).unwrap();
    assert_eq!(report.entries.len(), scan::MAX_RESULTS);
    assert!(report.capped);
    assert!(report.entries.iter().all(|e| e.system == "NES"));
}

#[test]
fn cli_folder_smoke_and_invalid_arguments() {
    let parse = |args: &[&str]| crate::options(args.iter().map(std::ffi::OsString::from));
    assert_eq!(
        parse(&["--folder", "a b", "--smoke"]).unwrap(),
        ("a b".into(), true)
    );
    assert_eq!(
        parse(&[]).unwrap(),
        (std::env::current_dir().unwrap(), false)
    );
    for args in [
        vec!["--folder"],
        vec!["--smoke", "--smoke"],
        vec!["--other"],
        vec!["--folder", "a", "--folder", "b"],
    ] {
        assert!(parse(&args).is_err());
    }
}

#[test]
fn pool_shutdown_and_full_are_reported_without_ui_blocking() {
    let sandbox = Sandbox::new().unwrap();
    let mut pool = LocalPool::new();
    let executor = Rc::new(Local(pool.spawner()));
    let workers = Rc::new(RefCell::new(BlockingPool::new(1, 1)));
    let (started, ready) = oneshot::channel();
    let (release, blocked) = oneshot::channel();
    let running = workers
        .borrow()
        .try_spawn(move || {
            started.send(()).unwrap();
            futures::executor::block_on(blocked).unwrap();
        })
        .unwrap();
    pool.run_until(ready).unwrap();
    let queued = workers.borrow().try_spawn(|| ()).unwrap();
    let model = Store::new(
        App::new(sandbox.dir.to_string_lossy().into_owned()),
        executor.clone(),
    );
    let _tokens = wiring::connect(&model, executor, wiring::scanner(workers.clone()));
    pool.run_until_stalled();
    assert!(model.snapshot().error.contains("queue is full"));
    assert!(!model.snapshot().busy);
    release.send(()).unwrap();
    pool.run_until(async {
        running.await.unwrap();
        queued.await.unwrap();
    });
    let _ = model
        .command_rescan(Binding::constant(String::new()))
        .invoke();
    pool.run_until(crate::smoke::until(model.bind(|s| s.busy, None), |busy| {
        !busy
    }));
    assert!(model.snapshot().error.is_empty());
    assert!(model.snapshot().entries.is_empty());
    workers.borrow_mut().shutdown();
    let _ = model
        .command_rescan(Binding::constant(String::new()))
        .invoke();
    pool.run_until_stalled();
    assert!(model.snapshot().error.contains("shut down"));
    assert!(!model.snapshot().busy);
    assert_eq!(std::fs::read_dir(&sandbox.dir).unwrap().count(), 0);
}

#[test]
fn close_before_poll_never_submits_filesystem_work() {
    let mut pool = LocalPool::new();
    let executor = Rc::new(Local(pool.spawner()));
    let model = Store::new(App::new("unused".into()), executor.clone());
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let scanner: wiring::Scanner = Rc::new(move |_, _| {
        let count = count.clone();
        Box::pin(async move {
            count.set(count.get() + 1);
            Ok(Report::default())
        })
    });
    let _tokens = wiring::connect(&model, executor, scanner);
    model.close();
    pool.run_until_stalled();
    assert_eq!(calls.get(), 0);
}

#[test]
fn reactions_do_not_keep_store_owner_alive() {
    let pool = LocalPool::new();
    let executor = Rc::new(Local(pool.spawner()));
    let model = Store::new(App::new("unused".into()), executor.clone());
    let binding = model.bind(|s| s.filter.clone(), Some(|s, value| s.filter = value));
    let scanner: wiring::Scanner = Rc::new(|_, _| Box::pin(futures::future::pending()));
    let _tokens = wiring::connect(&model, executor, scanner);
    drop(model);
    assert!(!binding.set("cannot retain the owner".into()));
}
