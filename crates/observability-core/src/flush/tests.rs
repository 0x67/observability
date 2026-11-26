use std::{collections::HashMap, sync::MutexGuard, thread, time::Duration};

use metrics::Unit;
use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};
use tokio::runtime::Runtime;

use super::*;
use crate::{
    gate::set_metrics_enabled,
    handoff::{count_msg, merge_local, record_latency_ns},
};

const WINDOW: Duration = Duration::from_secs(10);

/// Locks shared test state, turns the gate on, and drains what earlier tests left in the slots.
fn setup() -> MutexGuard<'static, ()> {
    let state = crate::test_state();
    set_metrics_enabled(true);
    let _ = drain_into(&mut LatencyWindow::default());
    state
}

/// A current-thread runtime with paused time: an idle runtime jumps to its next timer.
fn paused_runtime() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("current-thread runtime builds")
}

/// Starts a flusher on `rt` that publishes into `recorder` once per `WINDOW`.
fn start(rt: &Runtime, recorder: &DebuggingRecorder) -> Flusher {
    metrics::with_local_recorder(recorder, || {
        let _context = rt.enter();
        spawn_flusher("lat", "msgs", WINDOW)
    })
    .expect("flusher starts inside a runtime")
}

/// Runs paused time just past the next window, so the flusher's tick runs first.
fn next_window(rt: &Runtime) {
    rt.block_on(async { tokio::time::sleep(WINDOW + Duration::from_millis(1)).await });
}

/// Runs `work` on a thread that exits before this returns.
fn on_worker(work: fn()) {
    thread::spawn(work).join().expect("worker thread panicked");
}

type Snapshot = HashMap<String, (Option<Unit>, DebugValue)>;

/// Reads every metric once; the debugging recorder resets each value it reports.
fn snapshot(snapshotter: &Snapshotter) -> Snapshot {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .map(|(key, unit, _, value)| (key.key().name().to_owned(), (unit, value)))
        .collect()
}

fn gauge(snapshot: &Snapshot, name: &str) -> f64 {
    match snapshot.get(name) {
        Some((_, DebugValue::Gauge(value))) => value.0,
        other => panic!("{name}: expected a gauge, got {other:?}"),
    }
}

fn counter(snapshot: &Snapshot, name: &str) -> u64 {
    match snapshot.get(name) {
        Some((_, DebugValue::Counter(value))) => *value,
        other => panic!("{name}: expected a counter, got {other:?}"),
    }
}

fn unit(snapshot: &Snapshot, name: &str) -> Option<Unit> {
    snapshot.get(name).and_then(|(unit, _)| *unit)
}

fn assert_within_1pct(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= expected * 0.01,
        "{actual} is not within 1% of {expected}"
    );
}

/// 100 samples of 1 ms, one of 50 ms, and 7 messages, merged on a thread that exits.
fn first_window_work() {
    for _ in 0..100 {
        record_latency_ns(1_000_000);
    }
    record_latency_ns(50_000_000);
    for _ in 0..7 {
        count_msg();
    }
    merge_local();
}

/// Each window publishes quantiles and max in seconds through handles built once at start.
// @lat: [[tests#Tokio flusher#Publishes window quantiles in seconds]]
#[test]
fn publishes_window_quantiles_in_seconds() {
    let _state = setup();
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let rt = paused_runtime();
    let _flusher = start(&rt, &recorder);

    on_worker(first_window_work);
    next_window(&rt);
    let first = snapshot(&snapshotter);
    assert_within_1pct(gauge(&first, "lat.p50"), 0.001);
    assert_within_1pct(gauge(&first, "lat.max"), 0.05);
    assert_eq!(gauge(&first, "lat.samples"), 101.0);
    assert_eq!(counter(&first, "msgs"), 7);
    assert_eq!(unit(&first, "lat.p99"), Some(Unit::Seconds));
    assert_eq!(unit(&first, "lat.samples"), Some(Unit::Count));

    on_worker(|| {
        for _ in 0..10 {
            record_latency_ns(2_000_000);
        }
        merge_local();
    });
    next_window(&rt);
    let second = snapshot(&snapshotter);
    assert_within_1pct(gauge(&second, "lat.p50"), 0.002);
    assert_eq!(gauge(&second, "lat.samples"), 10.0);
}

/// A window with no samples sets the sample count to 0 and leaves the quantiles alone.
// @lat: [[tests#Tokio flusher#Empty window keeps last quantiles]]
#[test]
fn empty_window_keeps_last_quantiles() {
    let _state = setup();
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let rt = paused_runtime();
    let _flusher = start(&rt, &recorder);

    on_worker(first_window_work);
    next_window(&rt);
    // no snapshot here: reading would reset the gauges this test checks
    next_window(&rt);

    let after_idle = snapshot(&snapshotter);
    assert_eq!(gauge(&after_idle, "lat.samples"), 0.0);
    assert_within_1pct(gauge(&after_idle, "lat.p50"), 0.001);
}

/// Without a runtime the call fails and leaves no running flag behind.
// @lat: [[tests#Tokio flusher#Spawn outside a runtime returns NoRuntime]]
#[test]
fn spawn_outside_runtime_returns_no_runtime() {
    let _state = setup();
    assert_eq!(
        spawn_flusher("lat", "msgs", WINDOW).err(),
        Some(FlusherError::NoRuntime)
    );

    let rt = paused_runtime();
    let _context = rt.enter();
    assert!(
        spawn_flusher("lat", "msgs", WINDOW).is_ok(),
        "the failed call must not leave the running flag set"
    );
}

/// A second flusher is refused while the first one's handle is alive.
// @lat: [[tests#Tokio flusher#Second spawn while running is refused]]
#[test]
fn second_spawn_while_running_is_refused() {
    let _state = setup();
    let rt = paused_runtime();
    let _context = rt.enter();
    let _first = spawn_flusher("lat", "msgs", WINDOW).expect("first flusher starts");
    assert_eq!(
        spawn_flusher("lat", "msgs", WINDOW).err(),
        Some(FlusherError::AlreadyRunning)
    );
}

/// Dropping the handle stops publishing and frees the flag for a new flusher.
// @lat: [[tests#Tokio flusher#Dropping the handle allows a respawn]]
#[test]
fn dropping_handle_allows_respawn() {
    let _state = setup();
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let rt = paused_runtime();
    drop(start(&rt, &recorder));
    let _ = snapshot(&snapshotter);

    on_worker(|| {
        record_latency_ns(1_000);
        count_msg();
        merge_local();
    });
    rt.block_on(async { tokio::time::sleep(3 * WINDOW).await });
    assert_eq!(
        counter(&snapshot(&snapshotter), "msgs"),
        0,
        "an aborted flusher must not drain"
    );

    let _context = rt.enter();
    assert!(spawn_flusher("lat", "msgs", WINDOW).is_ok());
}

/// A zero interval is refused instead of panicking inside tokio.
// @lat: [[tests#Tokio flusher#Zero interval is refused]]
#[test]
fn zero_interval_is_refused() {
    let _state = setup();
    let rt = paused_runtime();
    let _context = rt.enter();
    assert_eq!(
        spawn_flusher("lat", "msgs", Duration::ZERO).err(),
        Some(FlusherError::ZeroInterval)
    );
}
