use std::io;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::jkbms::{
    ConnectionState, JkBmsConfigOptions, JkBmsConnection, JkBmsData, JkBmsDeviceInfo, JkBmsError,
    JkBmsEvents, JkBmsOperationalData, WriteCommand,
};

use super::support::protocol_mock::{JkBmsProtocolMock, ProtocolStep};
use super::support::transport_opener_mock::{JkBmsTransportOpenerMock, TransportOpenerStep};

use super::super::connection_manager::internals::{
    OPERATIONAL_POLL_INTERVAL, RECONNECT_BACKOFF, RECONNECT_BACKOFF_MAX, RECONNECT_THRESHOLD,
    WRITE_TTL,
};

// ── Dummy value constructors ──────────────────────────────────────────────────

fn dummy_device_info() -> JkBmsDeviceInfo {
    JkBmsDeviceInfo {
        model: String::from("Test"),
        hardware_version: String::from("1.0"),
        software_version: String::from("1.0"),
        serial_number: String::from("TEST001"),
        power_cycle_count: 1,
    }
}

fn dummy_config() -> JkBmsConfigOptions {
    JkBmsConfigOptions {
        cell_count: 16,
        charging_switch: true,
        balance_switch: true,
        battery_capacity_ah: 100.0,
        smart_sleep_voltage_v: 3.0,
        cell_undervoltage_protection_v: 2.8,
        cell_overvoltage_protection_v: 3.65,
        balance_trigger_voltage_v: 3.45,
    }
}

fn dummy_operational() -> JkBmsOperationalData {
    JkBmsOperationalData {
        cell_voltages_v: [3.3; 16],
        cell_resistances_ohm: [0.05; 16],
        total_voltage_v: 52.8,
        total_current_a: 5.0,
        soc_pct: 80,
        soh_pct: 100,
        capacity_remaining_ah: 80.0,
        total_cycle_capacity_ah: 1000.0,
        charging_cycles: 10,
        total_runtime_s: 3600,
        mos_temperature_c: 25.0,
        temperature_sensor_1_c: 22.0,
        temperature_sensor_2_c: 23.0,
        temperature_sensor_4_c: 21.0,
        temperature_sensor_5_c: 20.0,
        balancing_current_a: 0.0,
        balancing_active: false,
        charging_switch: true,
    }
}

fn hard_io() -> JkBmsError {
    JkBmsError::Io(io::Error::other("hard I/O error"))
}

fn soft_io() -> JkBmsError {
    JkBmsError::Io(io::Error::new(io::ErrorKind::TimedOut, "timed out"))
}

// ── Script-building helpers ───────────────────────────────────────────────────

fn resync_steps() -> Vec<ProtocolStep> {
    vec![
        ProtocolStep::DeviceInfo(Ok(dummy_device_info())),
        ProtocolStep::Config(Ok(dummy_config())),
        ProtocolStep::Operational(Ok(dummy_operational())),
    ]
}

fn alarm_steps() -> Vec<ProtocolStep> {
    vec![ProtocolStep::Alarms(Ok(0))]
}

fn write_ok_step() -> ProtocolStep {
    ProtocolStep::Write {
        command: WriteCommand::SetCharging(true),
        result: Ok(()),
    }
}

/// Successful write: switch write ok + config readback ok. `do_write` calls the
/// two protocol methods back-to-back atomically, so scripts must pair them.
fn write_ok_steps() -> Vec<ProtocolStep> {
    vec![write_ok_step(), ProtocolStep::Config(Ok(dummy_config()))]
}

fn write_fail_step() -> ProtocolStep {
    ProtocolStep::Write {
        command: WriteCommand::SetCharging(true),
        result: Err(hard_io()),
    }
}

// ── Test harness ──────────────────────────────────────────────────────────────

fn init_tracing() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .init();
    });
}

fn make_conn(
    steps: Vec<ProtocolStep>,
    opener_steps: Vec<TransportOpenerStep>,
) -> (JkBmsConnection, mpsc::UnboundedReceiver<JkBmsEvents>) {
    let opener = JkBmsTransportOpenerMock::new(opener_steps);
    make_conn_with_opener(steps, opener)
}

fn make_conn_with_opener(
    steps: Vec<ProtocolStep>,
    opener: JkBmsTransportOpenerMock,
) -> (JkBmsConnection, mpsc::UnboundedReceiver<JkBmsEvents>) {
    init_tracing();
    let protocol = Box::new(JkBmsProtocolMock::new(steps));
    super::super::connection::internals::new_with_deps(Box::new(opener), protocol)
}

/// Drain events until pred matches; panic after 64 events or 30 s virtual time.
/// `#[track_caller]` captures the call site so assertion messages point there.
#[track_caller]
fn recv_match<'a, P>(
    rx: &'a mut mpsc::UnboundedReceiver<JkBmsEvents>,
    pred: P,
    desc: &'static str,
) -> impl std::future::Future<Output = JkBmsEvents> + 'a
where
    P: Fn(&JkBmsEvents) -> bool + 'a,
{
    let loc = std::panic::Location::caller();
    recv_match_inner(rx, pred, desc, loc.file(), loc.line())
}

async fn recv_match_inner<P>(
    rx: &mut mpsc::UnboundedReceiver<JkBmsEvents>,
    pred: P,
    desc: &str,
    file: &'static str,
    line: u32,
) -> JkBmsEvents
where
    P: Fn(&JkBmsEvents) -> bool,
{
    for _ in 0..64 {
        let ev = tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .unwrap_or_else(|_| panic!("[{file}:{line}] recv_match({desc}): timed out"))
            .unwrap_or_else(|| panic!("[{file}:{line}] recv_match({desc}): channel closed"));
        if pred(&ev) {
            return ev;
        }
    }
    panic!("[{file}:{line}] recv_match({desc}): predicate never satisfied in 64 events");
}

fn is_connected(ev: &JkBmsEvents) -> bool {
    matches!(ev, JkBmsEvents::Connection(ConnectionState::Connected))
}
fn is_reconnecting(ev: &JkBmsEvents) -> bool {
    matches!(ev, JkBmsEvents::Connection(ConnectionState::Reconnecting))
}
fn is_disconnected(ev: &JkBmsEvents) -> bool {
    matches!(ev, JkBmsEvents::Connection(ConnectionState::Disconnected))
}
fn is_link_down(ev: &JkBmsEvents) -> bool {
    is_reconnecting(ev) || is_disconnected(ev)
}
fn is_operational(ev: &JkBmsEvents) -> bool {
    matches!(ev, JkBmsEvents::Data(JkBmsData::OperationalData(_)))
}
fn is_alarms(ev: &JkBmsEvents) -> bool {
    matches!(ev, JkBmsEvents::Data(JkBmsData::Alarms(_)))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Resync emits OperationalData before Connected; no byte sequences in this test.
#[tokio::test(start_paused = true)]
async fn operational_poll_round_trip() {
    let (_conn, mut rx) = make_conn(resync_steps(), vec![]);

    recv_match(&mut rx, is_operational, "is_operational").await;
    recv_match(&mut rx, is_connected, "is_connected").await;
}

/// Write handled while connected → WriteConfirmation on the same `seq`.
#[tokio::test(start_paused = true)]
async fn write_then_read_after_write_sequence() {
    let mut steps = resync_steps();
    steps.extend(alarm_steps());
    steps.extend(write_ok_steps());

    let (conn, mut rx) = make_conn(steps, vec![]);

    recv_match(&mut rx, is_connected, "is_connected").await;
    recv_match(&mut rx, is_alarms, "is_alarms").await;

    conn.write(WriteCommand::SetCharging(true), 1).unwrap();

    let ev = recv_match(
        &mut rx,
        |e| matches!(e, JkBmsEvents::WriteConfirmation { seq: 1, .. }),
        "WriteConfirmation{seq:1}",
    )
    .await;
    assert!(matches!(ev, JkBmsEvents::WriteConfirmation { seq: 1, .. }));
}

/// Hard I/O error during write → write lands in `pending_writes`; reconnect +
/// resync → write retried with the original seq preserved.
#[tokio::test(start_paused = true)]
async fn write_queued_on_disconnect_retried_after_reconnect() {
    let mut steps = resync_steps();
    steps.extend(alarm_steps());
    steps.push(write_fail_step());
    steps.extend(resync_steps());
    steps.extend(write_ok_steps());

    let (conn, mut rx) = make_conn(steps, vec![]);

    recv_match(&mut rx, is_connected, "is_connected").await;
    recv_match(&mut rx, is_alarms, "is_alarms").await;

    conn.write(WriteCommand::SetCharging(true), 99).unwrap();
    recv_match(&mut rx, is_reconnecting, "is_reconnecting").await;

    tokio::time::advance(RECONNECT_BACKOFF[0] + Duration::from_millis(1)).await;
    recv_match(&mut rx, is_connected, "is_connected").await;

    let ev = recv_match(
        &mut rx,
        |e| matches!(e, JkBmsEvents::WriteConfirmation { seq: 99, .. }),
        "WriteConfirmation{seq:99}",
    )
    .await;
    assert!(matches!(ev, JkBmsEvents::WriteConfirmation { seq: 99, .. }));
}

/// Same disconnect-mid-write, but WRITE_TTL elapses before resync →
/// write is evicted and WriteError is emitted; no retry.
#[tokio::test(start_paused = true)]
async fn write_queued_ttl_expires_dropped() {
    let mut steps = resync_steps();
    steps.extend(alarm_steps());
    steps.push(write_fail_step());
    steps.extend(resync_steps());
    steps.extend(alarm_steps()); // alarms due after WRITE_TTL elapsed

    let (conn, mut rx) = make_conn(steps, vec![]);

    recv_match(&mut rx, is_connected, "is_connected").await;
    recv_match(&mut rx, is_alarms, "is_alarms").await;

    conn.write(WriteCommand::SetCharging(true), 7).unwrap();
    recv_match(&mut rx, is_reconnecting, "is_reconnecting").await;

    tokio::time::advance(WRITE_TTL + RECONNECT_BACKOFF[0] + Duration::from_millis(1)).await;

    let ev = recv_match(
        &mut rx,
        |e| matches!(e, JkBmsEvents::WriteError { seq: 7 }),
        "WriteError{7}",
    )
    .await;
    assert!(matches!(ev, JkBmsEvents::WriteError { seq: 7 }));
}

/// Hard I/O error on alarm poll (not write) → immediate Reconnecting;
/// after backoff the actor reconnects, resyncs, and emits Connected again.
#[tokio::test(start_paused = true)]
async fn enodev_during_read_triggers_clean_reconnect() {
    let mut steps = resync_steps();
    steps.push(ProtocolStep::Alarms(Err(hard_io())));
    steps.extend(resync_steps());
    steps.extend(alarm_steps()); // alarms due after reconnect (last_alarms still None)

    let (_conn, mut rx) = make_conn(steps, vec![]);

    recv_match(&mut rx, is_connected, "is_connected").await;
    recv_match(&mut rx, is_reconnecting, "is_reconnecting").await;

    tokio::time::advance(RECONNECT_BACKOFF[0] + Duration::from_millis(1)).await;
    recv_match(&mut rx, is_connected, "is_connected").await;
}

/// Opener that always fails: 5 failed attempts emit `Reconnecting`; the 6th
/// (capped) attempt emits `Disconnected`. Stepped individually to verify the
/// exact event sequence rather than just the final state.
#[tokio::test(start_paused = true)]
async fn opener_always_fails_transitions_to_disconnected_after_cap() {
    let opener_steps: Vec<TransportOpenerStep> = (0..RECONNECT_BACKOFF.len() + 2)
        .map(|_| TransportOpenerStep::PathDisappears { for_ms: 10_000 })
        .collect();
    let opener = JkBmsTransportOpenerMock::new(opener_steps);
    let (_conn, mut rx) = make_conn_with_opener(vec![], opener);

    // Step through the full backoff schedule: 5 × Reconnecting, then Disconnected.
    tokio::task::yield_now().await;
    for &delay in &RECONNECT_BACKOFF {
        recv_match(&mut rx, is_reconnecting, "Reconnecting").await;
        tokio::time::advance(delay + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    recv_match(&mut rx, is_disconnected, "Disconnected after cap").await;
}

/// Regression: opener succeeds every attempt but resync always fails. Before
/// the reset-location fix this loop would reset `reconnect_attempt` on every
/// cycle and never exhaust the backoff schedule. Now 5 failures emit
/// `Reconnecting` and the 6th (capped) emits `Disconnected`.
#[tokio::test(start_paused = true)]
async fn resync_loop_eventually_emits_disconnected() {
    // 5 Reconnecting cycles + 1 Disconnected cycle.
    let cycles = RECONNECT_BACKOFF.len() + 1;
    let steps: Vec<ProtocolStep> = (0..cycles)
        .map(|_| ProtocolStep::DeviceInfo(Err(soft_io())))
        .collect();
    let (_conn, mut rx) = make_conn(steps, vec![]);

    // Step through the full backoff schedule: 5 × Reconnecting, then Disconnected.
    tokio::task::yield_now().await;
    for &delay in &RECONNECT_BACKOFF {
        recv_match(&mut rx, is_reconnecting, "Reconnecting").await;
        tokio::time::advance(delay + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }
    recv_match(
        &mut rx,
        is_disconnected,
        "Disconnected after 5 resync-failure cycles",
    )
    .await;
}

/// Reconnect backoff schedule: 250→1000→1750→2500→4500 ms, then capped at 5000 ms.
/// Verified by stepping each retry individually and inspecting open() timestamps.
#[tokio::test(start_paused = true)]
async fn reopen_backoff_caps() {
    let max_ms = RECONNECT_BACKOFF_MAX.as_millis() as u64;
    let delays: Vec<u64> = RECONNECT_BACKOFF
        .iter()
        .map(|d| d.as_millis() as u64)
        .chain([max_ms, max_ms])
        .collect();
    let n_failures = delays.len();

    let opener_steps: Vec<TransportOpenerStep> = delays
        .iter()
        .map(|&ms| TransportOpenerStep::PathDisappears { for_ms: ms })
        .collect();

    let mut protocol_steps = resync_steps();
    protocol_steps.extend(alarm_steps());

    let opener = JkBmsTransportOpenerMock::new(opener_steps);
    let handle = opener.handle();
    let (_conn, mut rx) = make_conn_with_opener(protocol_steps, opener);

    // Let the actor make its first open() at t=0 (immediately fails).
    tokio::task::yield_now().await;

    for &ms in &delays {
        tokio::time::advance(Duration::from_millis(ms + 1)).await;
        tokio::task::yield_now().await;
    }

    recv_match(&mut rx, is_connected, "is_connected").await;

    let open_ts = handle.open_timestamps();
    assert_eq!(
        open_ts.len(),
        n_failures + 1,
        "expected {} open calls ({n_failures} failures + 1 success), got {}",
        n_failures + 1,
        open_ts.len()
    );

    for (i, &exp_ms) in delays.iter().enumerate() {
        let actual_ms = (open_ts[i + 1] - open_ts[i]).as_millis() as u64;
        assert!(
            actual_ms >= exp_ms && actual_ms <= exp_ms + 2,
            "gap[{i}→{}]: expected ~{exp_ms} ms, got {actual_ms} ms",
            i + 1
        );
    }
}

/// RECONNECT_THRESHOLD consecutive soft failures across all poll types → forced
/// synthetic disconnect (stuck-state escalation, NFR-5 / ARCHITECTURE.md §10).
/// Any successful poll resets the counter, so escalation only fires when every
/// poll in every cycle fails.
///
/// With RECONNECT_THRESHOLD=5 and 2 failures per cycle (Operational + Alarms):
///   t=0  resync + first RunDataPolling: only Alarms due (last_alarms=None)
///   t=5  Operational timeout (cf=1), Alarms timeout (cf=2)
///   t=10 Operational timeout (cf=3), Alarms timeout (cf=4)
///   t=15 Operational timeout (cf=5 = RECONNECT_THRESHOLD) → disconnect; Alarms never polled
#[tokio::test(start_paused = true)]
async fn stuck_state_escalates() {
    let mut steps = resync_steps();
    steps.extend(alarm_steps()); // first RunDataPolling: only alarms due

    // full_cycles: cycles where both Operational and Alarms fail
    // final cycle: only Operational fails (disconnect happens before Alarms is reached)
    let full_cycles = (RECONNECT_THRESHOLD - 1) / 2;
    for _ in 0..full_cycles {
        steps.push(ProtocolStep::Operational(Err(soft_io())));
        steps.push(ProtocolStep::Alarms(Err(soft_io())));
    }
    steps.push(ProtocolStep::Operational(Err(soft_io())));
    // Alarms not reached: disconnect fires on cf = RECONNECT_THRESHOLD

    let (_conn, mut rx) = make_conn(steps, vec![]);

    recv_match(&mut rx, is_connected, "is_connected").await;
    recv_match(&mut rx, is_alarms, "is_alarms").await;

    for _ in 0..=full_cycles {
        tokio::time::advance(OPERATIONAL_POLL_INTERVAL + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }

    recv_match(
        &mut rx,
        is_link_down,
        "link down after stuck-state escalation",
    )
    .await;
}
