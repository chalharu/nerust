//! Controlled scanner replies measure CPU reactions, not filesystem or frames.
use super::*;
use std::time::Instant;

#[test]
#[ignore = "informational release workload; run scripts/measure.sh"]
fn library_rescan_projection() {
    let mut f = Fixture::new(Limits::default());
    let entries: Vec<_> = (0..256)
        .map(|index| entry(&format!("日本語のライブラリ{index:03}.nes")))
        .collect();
    // Establish all keyed models outside the timed rescans.
    f.reply(Ok(Report {
        entries: entries.clone(),
        capped: false,
    }));
    let original = f.model.snapshot().rows;
    assert_eq!(original.len(), 256);
    let generation = f.model.snapshot().generation;
    let mut identity_checks = 0;
    let mut samples = Vec::with_capacity(20);
    for cycle in 0..20 {
        // Selection really changes, then completion must reset it to zero.
        assert!(
            f.model
                .command_next(Binding::constant(String::new()))
                .invoke()
        );
        f.pool.run_until_stalled();
        let selected = f.model.snapshot();
        assert_eq!(selected.selection, 1);
        assert_eq!(selected.details, selected.entries[1].details());
        let mut next = entries.clone();
        next.rotate_left(cycle + 1); // Exercise keyed reuse across reordering.
        let expected = next.clone();
        let report = Report {
            entries: next,
            capped: false,
        };

        let start = Instant::now();
        f.rescan();
        f.reply(Ok(report));
        samples.push(start.elapsed());

        let state = f.model.snapshot();
        assert_eq!(state.revision, (cycle + 1) as u64);
        assert_eq!(state.generation, generation + (cycle + 1) as u64);
        assert!(!state.busy);
        assert!(state.pending.is_none());
        assert!(state.error.is_empty());
        assert!(!state.capped);
        assert_eq!(state.entries.as_ref(), &expected);
        assert_eq!(state.rows.len(), 256);
        assert_eq!(state.selection, 0);
        assert_eq!(state.details, expected[0].details());
        assert_eq!(
            state.status,
            "256 files — metadata only, no ROM reads or writes"
        );
        assert!(f.replies.borrow().is_empty());
        for (row, entry) in state.rows.iter().zip(&expected) {
            assert_eq!(row.key, entry.key());
            assert_eq!(row.model.snapshot().label, entry.label());
            let previous = original.iter().find(|old| old.key == row.key).unwrap();
            assert!(previous.model.ptr_eq(&row.model));
            identity_checks += 1;
        }
    }
    assert_eq!(identity_checks, 20 * 256);
    samples.sort_unstable();
    // Nearest-rank percentiles of command -> controlled reply -> LocalPool
    // drain, excluding fixture setup, report construction and state checks.
    println!(
        "WORKLOAD3 library release={} rescans=20 replies=20 rows_per_reply=256 identity_checks={} setup_replies=1 unit=rescan_reply_projection_cpu p50_ns={} p95_ns={} filesystem_scans=0 native_frames=0",
        !cfg!(debug_assertions),
        identity_checks,
        samples[9].as_nanos(),
        samples[18].as_nanos(),
    );
}
