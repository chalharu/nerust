//! Informational CPU workloads, not physical input or native frame latency.
use super::*;
use std::time::{Duration, Instant};

fn assert_draft(f: &Fixture, name: &str, volume: &str, invalid: bool) {
    let state = f.model.snapshot();
    assert_eq!(state.draft.name, name);
    assert_eq!(state.draft.volume, volume);
    assert_eq!(state.draft.parse().is_err(), invalid);
    assert_eq!(
        state.ui.error,
        state.draft.parse().err().unwrap_or_default()
    );
    assert_eq!(state.ui.menu_items[0].enabled, !invalid);
    assert!(state.ui.dirty);
    assert_eq!(state.ui.status, "Unsaved changes");
    assert!(state.ui == state.project());
    assert_eq!(state.saved, Config::default());
    assert!(state.pending.is_none());
    assert!(state.io_error.is_empty());
}

#[test]
#[ignore = "informational release workload; run scripts/measure.sh"]
fn settings_draft_projection() {
    let mut f = Fixture::new();
    // Keep the real save wiring, but make worker IO impossible. Fixture setup
    // and sandbox cleanup are outside every sample; no save is requested.
    f.workers.borrow_mut().shutdown();
    f.pool.run_until_stalled();
    let volume = f.model.bind(
        |s| s.draft.volume.clone(),
        Some(|s, value| s.draft.volume = value),
    );
    let projections = Rc::new(Cell::new(0));
    let count = projections.clone();
    let _projection = f
        .model
        .bind(|s| s.ui.clone(), None)
        .subscribe(Rc::new(move |_| {
            count.set(count.get() + 1);
        }));
    projections.set(0); // Exclude subscribe's initial delivery.
    let save_requests = Rc::new(Cell::new(0));
    let count = save_requests.clone();
    let _pending = f
        .model
        .bind(|s| s.pending.clone(), None)
        .subscribe(Rc::new(move |request| {
            if request.is_some() {
                count.set(count.get() + 1);
            }
        }));

    let mut samples = Vec::with_capacity(100);
    for cycle in 0..100 {
        let name = format!("日本語の設定下書き{cycle:03}");
        let previous_volume = volume.get();
        let invalid = if cycle % 2 == 0 { "" } else { "-" };
        let valid = format!("{cycle:03}"); // Preserve raw leading zeroes too.
        let mut elapsed = Duration::ZERO;

        let start = Instant::now();
        f.edit(&name);
        f.pool.run_until_stalled();
        elapsed += start.elapsed();
        assert_draft(&f, &name, &previous_volume, false);

        let text = invalid.to_string();
        let start = Instant::now();
        assert!(volume.set(text));
        f.pool.run_until_stalled();
        elapsed += start.elapsed();
        assert_eq!(volume.get(), invalid); // Editor binding retains invalid text.
        assert_draft(&f, &name, invalid, true);

        let text = valid.clone();
        let start = Instant::now();
        assert!(volume.set(text));
        f.pool.run_until_stalled();
        elapsed += start.elapsed();
        assert_eq!(volume.get(), valid);
        assert_draft(&f, &name, &valid, false);
        samples.push(elapsed);
    }
    // First edit makes the draft dirty; later name edits leave the saved-only
    // summary unchanged. Each invalid/valid pair publishes two UI changes.
    assert_eq!(projections.get(), 1 + 100 * 2);
    assert_eq!(save_requests.get(), 0);
    assert!(!f.sandbox.path().exists());
    assert_eq!(std::fs::read_dir(&f.sandbox.dir).unwrap().count(), 0);
    samples.sort_unstable();
    // Nearest-rank percentiles. Each sample sums three update-to-drain spans;
    // validation assertions and input construction are outside those spans.
    println!(
        "WORKLOAD3 settings release={} cycles=100 updates=300 invalid_edits=100 projections={} save_requests={} unit=three_update_cycle_cpu p50_ns={} p95_ns={} native_frames=0",
        !cfg!(debug_assertions),
        projections.get(),
        save_requests.get(),
        samples[49].as_nanos(),
        samples[94].as_nanos(),
    );
}
