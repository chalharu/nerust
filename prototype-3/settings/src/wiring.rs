//! Effect/UI ownership is outside the pure application state.
use crate::{
    backend,
    model::{App, AppCommands},
    persistence,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use wakui2_runtime::{BlockingPool, Policy, Store, Subscription};

pub fn project(model: &Store<App>) -> Subscription {
    // Excludes ui itself: a projection update cannot feed back into its source.
    model.watch(
        model.bind(
            |s| {
                (
                    s.saved.clone(),
                    s.draft.clone(),
                    s.pending.clone(),
                    s.exit,
                    s.io_error.clone(),
                )
            },
            None,
        ),
        |target, _| {
            let state = target.snapshot();
            let next = state.project();
            if next != state.ui {
                let _ = target.update(move |s| {
                    if s.ui != next {
                        s.ui = next;
                    }
                });
            }
        },
    )
}

pub fn saves(
    model: &Store<App>,
    workers: Rc<RefCell<BlockingPool>>,
    path: PathBuf,
) -> Subscription {
    model.watch(
        model.bind(|s| s.pending.clone(), None),
        move |target, request| {
            let Some(request) = request else { return };
            let workers = workers.clone();
            let path = path.clone();
            let config = request.config.clone();
            // No try_spawn before effect acceptance: a rejected/dropped future does
            // not submit any filesystem work. The borrow also ends before await.
            if !target.effect(
                Policy::Serial,
                "settings-save",
                async move {
                    let task = workers
                        .borrow()
                        .try_spawn(move || persistence::save(&path, &config))
                        .map_err(|error| error.to_string())?;
                    task.await.map_err(|error| error.to_string())?
                },
                move |state, result| state.complete(request, result),
            ) {
                let _ = target.update(|s| {
                    if let Some(request) = s.pending.clone() {
                        s.complete(
                            request,
                            Err("Save was not accepted: task capacity exceeded".into()),
                        );
                    }
                });
            }
        },
    )
}

pub fn view(model: Store<App>, workers: Rc<RefCell<BlockingPool>>, path: PathBuf) -> backend::View {
    let projection = project(&model);
    let saving = saves(&model, workers, path);
    let view = wakui2_macros::view!("ui/settings.yaml", model, backend);
    view.keep(projection);
    view.keep(saving);
    let owner = model.clone();
    view.on_close_request(Rc::new(move || App::request_exit(owner.clone())));
    // View -> subscription -> WeakView, never View -> callback -> View.
    let weak = view.downgrade();
    view.keep(
        model
            .bind(App::close_ready, None)
            .subscribe(Rc::new(move |ready| {
                if ready && let Some(view) = weak.upgrade() {
                    view.close();
                }
            })),
    );
    view.on_close(Rc::new(move || model.close()));
    view
}
