//! Informational debug-build observations, not a native frame benchmark.
use panechord_runtime::{Executor, Store};
use std::{cell::Cell, hint::black_box, rc::Rc, time::Instant};

struct NoTasks;
impl Executor for NoTasks {
    fn spawn(&self, _: futures::future::LocalBoxFuture<'static, ()>) {
        panic!("the scaling probe must not schedule tasks");
    }
}
struct State {
    version: usize,
    payload: Vec<u8>,
    clones: Rc<Cell<usize>>,
}
impl Clone for State {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            version: self.version,
            payload: self.payload.clone(),
            clones: self.clones.clone(),
        }
    }
}

fn main() {
    const READS: usize = 50_000;
    const UPDATES: usize = 20;
    for payload_bytes in [0, 1024 * 1024, 16 * 1024 * 1024] {
        for subscribers in [1, 100, 1000] {
            let clones = Rc::new(Cell::new(0));
            let model = Store::new(
                State {
                    version: 0,
                    payload: vec![1; payload_bytes],
                    clones: clones.clone(),
                },
                Rc::new(NoTasks),
            );
            let scalar = model.bind(|s| s.version, None);
            let delivered = Rc::new(Cell::new(0));
            let tokens: Vec<_> = (0..subscribers)
                .map(|_| {
                    let delivered = delivered.clone();
                    scalar.subscribe(Rc::new(move |_| delivered.set(delivered.get() + 1)))
                })
                .collect();
            let started = Instant::now();
            for _ in 0..READS {
                black_box(scalar.get());
            }
            let read_us = started.elapsed().as_micros();
            assert_eq!(clones.get(), 0, "reads and subscription must not clone S");
            let mut timings = Vec::with_capacity(UPDATES);
            for version in 1..=UPDATES {
                let started = Instant::now();
                assert!(model.update(move |s| s.version = version));
                timings.push(started.elapsed());
            }
            timings.sort_unstable();
            assert_eq!(scalar.get(), UPDATES);
            assert_eq!(clones.get(), UPDATES, "one full clone per update remains");
            assert_eq!(delivered.get(), (UPDATES + 1) * subscribers);
            println!(
                "SCALING3 payload_bytes={payload_bytes} subscribers={subscribers} reads={READS} read_us={read_us} updates={UPDATES} update_p50_us={} update_p95_us={} state_clones={} notifications={}",
                timings[UPDATES / 2 - 1].as_micros(),
                timings[(UPDATES * 95).div_ceil(100) - 1].as_micros(),
                clones.get(),
                delivered.get(),
            );
            drop(tokens);
            model.close();
        }
    }
    println!("SCALING3 PASS: read clones=0, update clones=1, notification counts exact");
    scoped();
}

struct SharedState {
    version: usize,
    payload: Rc<Vec<u8>>,
    clones: Rc<Cell<usize>>,
}
impl Clone for SharedState {
    fn clone(&self) -> Self {
        self.clones.set(self.clones.get() + 1);
        Self {
            version: self.version,
            payload: self.payload.clone(),
            clones: self.clones.clone(),
        }
    }
}
fn scoped() {
    const UPDATES: usize = 200;
    for subscribers in [1, 100, 1000, 10_000] {
        let clones = Rc::new(Cell::new(0));
        let projections = Rc::new(Cell::new(0));
        let model = Store::new(
            SharedState {
                version: 0,
                payload: Rc::new(vec![1; 16 * 1024 * 1024]),
                clones: clones.clone(),
            },
            Rc::new(NoTasks),
        );
        let tokens: Vec<_> = (0..subscribers)
            .map(|_| {
                let projections = projections.clone();
                model
                    .bind_scoped_explicit(
                        "payload",
                        move |state| {
                            projections.set(projections.get() + 1);
                            state.payload.len()
                        },
                        None,
                    )
                    .subscribe(Rc::new(|_| {}))
            })
            .collect();
        let delivered = Rc::new(Cell::new(0));
        let output = delivered.clone();
        let changed = model
            .bind_scoped_explicit("version", |state| state.version, None)
            .subscribe(Rc::new(move |_| output.set(output.get() + 1)));
        let mut timings = Vec::new();
        for version in 1..=UPDATES {
            let started = Instant::now();
            assert!(
                model.update_scoped_explicit(&["version"], move |state| Some(SharedState {
                    version,
                    payload: state.payload.clone(),
                    clones: state.clones.clone()
                }))
            );
            timings.push(started.elapsed().as_nanos());
            assert!(model.update_scoped_explicit(&["version"], |_| None));
        }
        timings.sort_unstable();
        assert_eq!(clones.get(), 0);
        assert_eq!(projections.get(), subscribers);
        assert_eq!(delivered.get(), UPDATES + 1);
        println!(
            "SCOPED3 subscribers={subscribers} updates={UPDATES} p50_ns={} p95_ns={} state_clones=0 unrelated_reevaluations=0",
            timings[UPDATES / 2 - 1],
            timings[(UPDATES * 95).div_ceil(100) - 1]
        );
        drop((tokens, changed));
    }
    println!("SCOPED3 PASS clone-free-replacement indexed-dependencies noop-generation");
}
