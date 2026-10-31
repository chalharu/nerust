//! Reactions borrow the runtime-supplied Store, never capture a strong owner.
use crate::{
    backend,
    model::{App, AppCommands, Row},
    scan,
};
use futures::future::LocalBoxFuture;
use panechord_runtime::{BlockingPool, Executor, Keyed, Policy, Store, Subscription};
use std::{cell::RefCell, rc::Rc};

pub type Scanner =
    Rc<dyn Fn(String, String) -> LocalBoxFuture<'static, Result<scan::Report, String>>>;

pub fn scanner(workers: Rc<RefCell<BlockingPool>>) -> Scanner {
    Rc::new(move |folder, filter| {
        let workers = workers.clone();
        Box::pin(async move {
            // This body is not polled until effect acceptance. A rejected task
            // cannot submit orphan work; no RefCell borrow crosses await.
            let task = workers
                .borrow()
                .try_spawn(move || scan::scan(folder.into(), filter))
                .map_err(|error| error.to_string())?;
            task.await.map_err(|error| error.to_string())?
        })
    })
}

pub fn connect(model: &Store<App>, executor: Rc<dyn Executor>, scan: Scanner) -> Vec<Subscription> {
    let requests = model.watch(model.bind(App::query, None), |model, query| {
        let _ = model.update(move |state| state.begin(query));
    });
    let tasks = model.watch(
        model.bind(|s| s.pending.clone(), None),
        move |model, request| {
            let Some(request) = request else {
                return;
            };
            let completion = request.clone();
            // Constructing the future is cheap; scanner submission is deferred to poll.
            let future = scan(request.query.folder.clone(), request.query.filter.clone());
            if !model.effect(
                Policy::Latest,
                "library-scan",
                future,
                move |state, result| {
                    state.complete(completion, result);
                },
            ) {
                let _ = model.update(move |state| {
                    state.complete(
                        request,
                        Err("Scan task was not accepted: task capacity exceeded".into()),
                    )
                });
            }
        },
    );
    let projection = model.watch(
        model.bind(
            |s| {
                (
                    s.entries.clone(),
                    s.selection,
                    s.busy,
                    s.error.clone(),
                    s.capped,
                )
            },
            None,
        ),
        move |model, (entries, selection, busy, error, capped)| {
            let previous = model.snapshot().rows;
            let rows = entries
                .iter()
                .map(|entry| {
                    let key = entry.key();
                    previous
                        .iter()
                        .find(|row| row.key == key && row.model.snapshot().label == entry.label())
                        .cloned()
                        .unwrap_or_else(|| {
                            Keyed::new(
                                key,
                                Store::new(
                                    Row {
                                        label: entry.label(),
                                    },
                                    executor.clone(),
                                ),
                            )
                        })
                })
                .collect();
            let details = entries
                .get(selection)
                .map_or_else(|| "No file selected".into(), scan::Entry::details);
            let status = if busy {
                "Scanning… edits remain responsive".into()
            } else if !error.is_empty() {
                "Scan failed — edit folder or rescan to retry".into()
            } else {
                format!(
                    "{} files{} — metadata only, no ROM reads or writes",
                    entries.len(),
                    if capped {
                        " (limit reached; refine filter)"
                    } else {
                        ""
                    }
                )
            };
            let _ = model.update(move |state| {
                state.rows = rows;
                state.details = details;
                state.status = status;
            });
        },
    );
    vec![requests, tasks, projection]
}

pub fn view(
    model: Store<App>,
    executor: Rc<dyn Executor>,
    workers: Rc<RefCell<BlockingPool>>,
) -> backend::View {
    let tokens = connect(&model, executor, scanner(workers));
    let view = panechord_macros::view!("ui/library.yaml", model, backend);
    for token in tokens {
        view.keep(token);
    }
    let owner = model.clone();
    view.on_close_request(Rc::new(move || {
        let _ = owner.update(|state| state.closing = true);
    }));
    let weak = view.downgrade();
    view.keep(
        model.watch(model.bind(|s| s.closing, None), move |_, closing| {
            if closing && let Some(view) = weak.upgrade() {
                view.close();
            }
        }),
    );
    // Explicit app owner: bindings and reaction callbacks are not owners.
    view.on_close(Rc::new(move || model.close()));
    view
}
