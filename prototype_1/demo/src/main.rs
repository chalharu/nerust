//! Dedicated proof app: deliberately has no dependency on nerust or legacy wakui.
use demo_contract::{Input, Settings, State};
use futures::{
    channel::oneshot,
    executor::{LocalPool, LocalSpawner},
    task::LocalSpawnExt,
};
use std::{cell::RefCell, fs, io::Write, path::PathBuf, rc::Rc};
use wakui_runtime::{Binding, BlockingPool, Executor, ListItem, ModelHandle, UpdateCtx, ViewModel};

#[cfg(feature = "backend-gtk")]
use wakui_gtk as backend;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use wakui_tao as backend;

#[cfg(all(feature = "backend-gtk", feature = "backend-tao"))]
compile_error!("select only one backend");

mod generated {
    include!(concat!(env!("OUT_DIR"), "/view_generated.rs"));
}

struct Model {
    state: State,
    pool: Rc<RefCell<BlockingPool>>,
    save_file: PathBuf,
    next_id: u64,
}
enum Message {
    Saved(Result<(), String>),
}

impl Model {
    fn new(pool: Rc<RefCell<BlockingPool>>, save_file: PathBuf) -> Self {
        Self {
            state: State {
                name: "Wakui".into(),
                status: "Ready".into(),
                busy: false,
                saved: 0,
                rows: vec![
                    ListItem {
                        key: "1".into(),
                        text: "First item".into(),
                    },
                    ListItem {
                        key: "2".into(),
                        text: "Second item".into(),
                    },
                ],
            },
            pool,
            save_file,
            next_id: 3,
        }
    }
}

impl ViewModel for Model {
    type Contract = Settings;
    type Message = Message;
    fn state(&self) -> State {
        self.state.clone()
    }
    fn update(&mut self, input: Input, cx: &mut UpdateCtx<Message>) {
        match input {
            Input::Name(name) => self.state.name = name,
            Input::Add => {
                if self.state.rows.len() >= 100 || self.state.name.trim().is_empty() {
                    return;
                }
                self.state.rows.push(ListItem {
                    key: self.next_id.to_string(),
                    text: self.state.name.clone(),
                });
                self.next_id = self.next_id.checked_add(1).expect("item ID exhausted");
            }
            Input::Reverse => self.state.rows.reverse(),
            Input::Clear => self.state.rows.clear(),
            Input::Save => {
                if self.state.busy || self.state.name.trim().is_empty() {
                    return;
                }
                let path = self.save_file.clone();
                let name = self.state.name.clone();
                let task = self
                    .pool
                    .borrow()
                    .try_spawn(move || atomic_save(&path, name.as_bytes()));
                match task {
                    Ok(task) => {
                        self.state.busy = true;
                        self.state.status = "Saving…".into();
                        // Command gate prevents replacement of an in-progress write.
                        cx.latest("save", async move {
                            Message::Saved(task.await.map_err(|e| e.to_string()).and_then(|r| r))
                        });
                    }
                    Err(error) => self.state.status = format!("Error: {error}"),
                }
            }
        }
    }
    fn complete(&mut self, message: Message, _: &mut UpdateCtx<Message>) {
        match message {
            Message::Saved(result) => {
                self.state.busy = false;
                self.state.status = match result {
                    Ok(()) => {
                        self.state.saved += 1;
                        format!("Saved ({})", self.state.saved)
                    }
                    Err(error) => format!("Error: {error}"),
                };
            }
        }
    }
}

fn atomic_save(path: &std::path::Path, value: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(value)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    // Only remove a file this invocation created; an existing temp is not ours.
    if result.is_err()
        && result
            .as_ref()
            .err()
            .is_some_and(|e| e.kind() != std::io::ErrorKind::AlreadyExists)
    {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("{}: {e}", path.display()))
}

async fn until<T: Clone + 'static>(
    binding: Binding<T>,
    predicate: impl Fn(&T) -> bool + 'static,
) -> T {
    let (sender, receiver) = oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let token = binding.subscribe(Rc::new(move |value| {
        if predicate(&value)
            && let Some(sender) = sender.borrow_mut().take()
        {
            let _ = sender.send(value);
        }
    }));
    let value = receiver
        .await
        .expect("observed state survives until completion");
    drop(token);
    value
}

async fn native_smoke(view: backend::View, model: ModelHandle<Model>, path: PathBuf) {
    assert_eq!(view.text("echo").as_deref(), Some("Wakui"));
    view.edit("name", "Ada").unwrap();
    assert_eq!(model.snapshot().name, "Ada");
    assert_eq!(view.text("echo").as_deref(), Some("Ada"));
    assert!(view.edit("name_preview", "must not change").is_err());
    assert_eq!(view.text("name_preview").as_deref(), Some("Ada"));
    view.click("add").unwrap();
    assert_eq!(view.item_keys("rows"), ["1", "2", "3"]);
    view.click("reverse").unwrap();
    assert_eq!(view.item_keys("rows"), ["3", "2", "1"]);
    view.activate_menu("save_menu").unwrap();
    assert!(model.snapshot().busy);
    assert!(
        view.click("save_button").is_err(),
        "shared Command must reject duplicate save"
    );
    until(model.bind(&demo_contract::BUSY), |busy| !busy).await;
    assert_eq!(model.snapshot().saved, 1, "{}", model.snapshot().status);
    assert_eq!(view.text("status").as_deref(), Some("Saved (1)"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "Ada");
    view.edit("name", "Grace").unwrap();
    view.click("save_button").unwrap();
    until(model.bind(&demo_contract::BUSY), |busy| !busy).await;
    assert_eq!(model.snapshot().saved, 2);
    assert_eq!(fs::read_to_string(&path).unwrap(), "Grace");
    view.click("clear").unwrap();
    assert!(view.item_keys("rows").is_empty());
    view.close();
    view.close();
    assert!(model.is_closed());
    assert!(view.edit("name", "closed").is_err());
    println!(
        "SMOKE PASS backend={} saves=2 bindings=TwoWay menu=shared items=keyed closed=true",
        generated::BACKEND
    );
}

struct LocalExecutor(LocalSpawner);
impl Executor for LocalExecutor {
    fn spawn(&self, future: futures::future::LocalBoxFuture<'static, ()>) {
        self.0.spawn_local(future).unwrap();
    }
}

fn headless(pool: Rc<RefCell<BlockingPool>>, path: PathBuf) {
    let mut executor = LocalPool::new();
    let model = ModelHandle::new(
        Model::new(pool, path.clone()),
        Rc::new(LocalExecutor(executor.spawner())),
    );
    assert!(model.bind(&demo_contract::NAME).set("Headless".into()));
    let save = model.command(&demo_contract::SAVE);
    assert!(save.invoke());
    assert!(!save.invoke());
    executor.run_until(until(model.bind(&demo_contract::BUSY), |busy| !busy));
    assert_eq!(model.snapshot().saved, 1, "{}", model.snapshot().status);
    assert_eq!(fs::read_to_string(path).unwrap(), "Headless");
    model.close();
    executor.run_until_stalled();
    println!("HEADLESS PASS saves=1 typed=true async=true");
}

fn main() -> Result<(), String> {
    let mut smoke = false;
    let mut no_window = false;
    let mut save_file = std::env::temp_dir().join(format!("wakui-demo-{}.txt", std::process::id()));
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke" => smoke = true,
            "--headless" => no_window = true,
            "--save-file" => {
                save_file = PathBuf::from(args.next().ok_or("--save-file needs a path")?)
            }
            "--help" | "-h" => {
                println!("wakui-demo [--smoke | --headless] [--save-file PATH]");
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    let pool = Rc::new(RefCell::new(BlockingPool::new(2, 8)));
    let result = if no_window {
        headless(pool.clone(), save_file);
        Ok(())
    } else {
        let workers = pool.clone();
        backend::run(move |executor| {
            let model = ModelHandle::new(Model::new(workers, save_file.clone()), executor.clone());
            let view = generated::create(&model);
            let owner = model.clone();
            view.on_close(Rc::new(move || owner.close()));
            if smoke {
                executor.spawn(Box::pin(native_smoke(view.clone(), model, save_file)));
            }
            view
        })
    };
    // All UI resources/tasks are closed; joining is deliberately outside the driver.
    pool.borrow_mut().shutdown();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "wakui-save-tests-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn atomic_save_replaces_the_destination_and_removes_its_temp() {
        let directory = Directory::new();
        let path = directory.0.join("settings.txt");
        atomic_save(&path, b"first").unwrap();
        atomic_save(&path, b"second").unwrap();
        assert_eq!(fs::read(path).unwrap(), b"second");
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn existing_temp_is_neither_overwritten_nor_removed() {
        let directory = Directory::new();
        let path = directory.0.join("settings.txt");
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&path, b"original").unwrap();
        fs::write(&temp, b"not owned by this invocation").unwrap();
        assert!(atomic_save(&path, b"replacement").is_err());
        assert_eq!(fs::read(path).unwrap(), b"original");
        assert_eq!(fs::read(temp).unwrap(), b"not owned by this invocation");
    }

    #[test]
    fn save_error_is_a_model_result_and_releases_command_gate() {
        let directory = Directory::new();
        let path = directory.0.join("missing-parent/settings.txt");
        let workers = Rc::new(RefCell::new(BlockingPool::new(1, 2)));
        let mut executor = LocalPool::new();
        let model = ModelHandle::new(
            Model::new(workers.clone(), path),
            Rc::new(LocalExecutor(executor.spawner())),
        );
        let save = model.command(&demo_contract::SAVE);
        assert!(save.invoke());
        executor.run_until(until(model.bind(&demo_contract::BUSY), |busy| !busy));
        assert_eq!(model.snapshot().saved, 0);
        assert!(model.snapshot().status.starts_with("Error:"));
        assert!(save.enabled.get());
        model.close();
        executor.run_until_stalled();
        workers.borrow_mut().shutdown();
    }

    #[test]
    fn blank_input_disables_save_without_starting_work() {
        let directory = Directory::new();
        let workers = Rc::new(RefCell::new(BlockingPool::new(1, 1)));
        let executor = LocalPool::new();
        let model = ModelHandle::new(
            Model::new(workers.clone(), directory.0.join("settings.txt")),
            Rc::new(LocalExecutor(executor.spawner())),
        );
        model.bind(&demo_contract::NAME).set("   ".into());
        assert!(!model.command(&demo_contract::SAVE).invoke());
        assert!(!model.snapshot().busy);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
        model.close();
        workers.borrow_mut().shutdown();
    }
}
