use std::time::Duration;

use tokio::sync::mpsc;

use super::super::config::MqttConfig;
use super::super::connection::internals::new_with_factory;
use super::super::connection_manager::internals::{INITIAL_BACKOFF, MAX_BACKOFF, STABLE_THRESHOLD};
use super::super::connection_manager::{MqttCommand, MqttConnectionManager};
use super::super::events::MqttEvents;
use super::super::mqttc_wrapper::IMqttClientFactory;
use super::support::mqttc_mock::{
    MqttClientFactoryMock, connack_step, delay_step, disconnect_step, io_error_step, make_session,
    publish_step, test_config,
};

fn new_manager(
    config: MqttConfig,
    factory: Box<dyn IMqttClientFactory>,
) -> (
    mpsc::Sender<MqttCommand>,
    mpsc::UnboundedReceiver<MqttEvents>,
) {
    let (commands_tx, commands_rx) = mpsc::channel(32);
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let manager = MqttConnectionManager::new(config, commands_rx, events_tx, factory);
    tokio::spawn(manager.run());
    (commands_tx, events_rx)
}

fn init_tracing() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
    });
}

/// Drain `events_rx` until `pred` is true; panic after 64 events or 30 s.
#[track_caller]
fn recv_match<'a, P>(
    rx: &'a mut tokio::sync::mpsc::UnboundedReceiver<MqttEvents>,
    pred: P,
    desc: &'static str,
) -> impl std::future::Future<Output = MqttEvents> + 'a
where
    P: Fn(&MqttEvents) -> bool + 'a,
{
    let loc = std::panic::Location::caller();
    recv_match_inner(rx, pred, desc, loc.file(), loc.line())
}

async fn recv_match_inner<P>(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<MqttEvents>,
    pred: P,
    desc: &str,
    file: &'static str,
    line: u32,
) -> MqttEvents
where
    P: Fn(&MqttEvents) -> bool,
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

fn is_broker_connected(e: &MqttEvents) -> bool {
    matches!(e, MqttEvents::BrokerConnected)
}
fn is_broker_disconnected(e: &MqttEvents) -> bool {
    matches!(e, MqttEvents::BrokerDisconnected)
}

// ═════════════════════════════════════════════════════════════════════════════
// Group 1 — Event routing: eventloop events → MqttEvents
// ═════════════════════════════════════════════════════════════════════════════

/// ConnAck fires → coordinator receives BrokerConnected.
#[tokio::test(start_paused = true)]
async fn connack_emits_broker_connected() {
    init_tracing();
    let (client, eventloop, _handle) = make_session(vec![connack_step(), io_error_step()]);
    let (factory, _fh) = MqttClientFactoryMock::new(vec![(client, eventloop)]);
    let (_cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;
}

/// ConnAck then Disconnect packet → BrokerConnected, then BrokerDisconnected.
#[tokio::test(start_paused = true)]
async fn disconnect_after_connack_emits_broker_disconnected() {
    init_tracing();
    let (client, eventloop, _handle) = make_session(vec![connack_step(), disconnect_step()]);
    let (factory, _fh) = MqttClientFactoryMock::new(vec![(client, eventloop)]);
    let (_cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;
    recv_match(&mut events_rx, is_broker_disconnected, "BrokerDisconnected").await;
}

/// Error before ConnAck → no BrokerConnected, no BrokerDisconnected.
/// After the first session fails, we advance time past backoff and then drop
/// commands_tx to exit the run loop; then assert no events were emitted.
#[tokio::test(start_paused = true)]
async fn error_before_connack_no_events() {
    init_tracing();
    let (client, eventloop, _handle) = make_session(vec![io_error_step()]);
    // Second session parks forever (we'll kill the manager via cmd_tx drop).
    let (client2, eventloop2, _h2) = make_session(vec![]);
    let (factory, _fh) =
        MqttClientFactoryMock::new(vec![(client, eventloop), (client2, eventloop2)]);
    let (cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    // Advance past the backoff so manager reaches session 2 and parks.
    tokio::time::advance(INITIAL_BACKOFF + Duration::from_millis(1)).await;
    // Drop commands_tx → manager sees HandleDropped and exits.
    drop(cmd_tx);

    let mut saw_connected = false;
    let mut saw_disconnected = false;
    while let Ok(ev) = tokio::time::timeout(Duration::from_millis(50), events_rx.recv()).await {
        match ev {
            Some(MqttEvents::BrokerConnected) => saw_connected = true,
            Some(MqttEvents::BrokerDisconnected) => saw_disconnected = true,
            _ => {}
        }
    }
    assert!(
        !saw_connected,
        "BrokerConnected should not be emitted without a ConnAck"
    );
    assert!(
        !saw_disconnected,
        "BrokerDisconnected requires a prior ConnAck"
    );
}

/// Incoming Publish on a set topic → MqttEvents::Incoming forwarded.
#[tokio::test(start_paused = true)]
async fn incoming_publish_forwarded_as_incoming_event() {
    use super::super::events::MqttEvents;
    use super::super::inbound::IncomingRequest;

    init_tracing();
    let (client, eventloop, _handle) = make_session(vec![
        connack_step(),
        publish_step("testbms/charging/set", b"ON"),
        io_error_step(),
    ]);
    let (factory, _fh) = MqttClientFactoryMock::new(vec![(client, eventloop)]);
    let (_cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;
    let ev = recv_match(
        &mut events_rx,
        |e| matches!(e, MqttEvents::Incoming(_)),
        "Incoming",
    )
    .await;
    assert!(
        matches!(ev, MqttEvents::Incoming(IncomingRequest::SetCharging(true))),
        "expected SetCharging(true), got {ev:?}",
    );
}

/// Error within FAST_FAIL_WINDOW of ConnAck still emits BrokerDisconnected
/// (fast-fail only changes log level, not the event contract).
#[tokio::test(start_paused = true)]
async fn fast_fail_still_emits_broker_disconnected() {
    init_tracing();
    let (client, eventloop, _handle) = make_session(vec![connack_step(), io_error_step()]);
    let (factory, _fh) = MqttClientFactoryMock::new(vec![(client, eventloop)]);
    let (_cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;
    recv_match(&mut events_rx, is_broker_disconnected, "BrokerDisconnected").await;
}

// ═════════════════════════════════════════════════════════════════════════════
// Group 2 — Dispatch: IMqttConnection public methods → IMqttClient calls
// Each test drives the full actor: sends a command through the public interface
// and asserts on what the mock client recorded.
// ═════════════════════════════════════════════════════════════════════════════

use super::super::connection::IMqttConnection;

async fn setup_connected_session(
    steps: Vec<super::support::mqttc_mock::EventLoopStep>,
) -> (
    impl IMqttConnection,
    tokio::sync::mpsc::UnboundedReceiver<MqttEvents>,
    super::support::mqttc_mock::MqttClientMockHandle,
) {
    let (client, eventloop, client_handle) = make_session(steps);
    let (factory, _fh) = MqttClientFactoryMock::new(vec![(client, eventloop)]);
    let (conn, mut events_rx) = new_with_factory(test_config(), Box::new(factory));
    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;
    (conn, events_rx, client_handle)
}

#[tokio::test(start_paused = true)]
async fn publish_availability_online_publishes_correct_call() {
    let (conn, _rx, client_handle) = setup_connected_session(vec![connack_step()]).await;
    conn.publish_availability(true).unwrap();
    tokio::task::yield_now().await;

    let pubs = client_handle.publishes();
    assert_eq!(pubs.len(), 1);
    assert_eq!(pubs[0].topic, "testbms/availability");
    assert_eq!(pubs[0].payload, b"online");
    assert!(pubs[0].retain, "availability must be retained");
    assert_eq!(pubs[0].qos, rumqttc::QoS::AtLeastOnce);
}

#[tokio::test(start_paused = true)]
async fn publish_availability_offline_sends_offline_payload() {
    let (conn, _rx, client_handle) = setup_connected_session(vec![connack_step()]).await;
    conn.publish_availability(false).unwrap();
    tokio::task::yield_now().await;

    let pubs = client_handle.publishes();
    assert_eq!(pubs.len(), 1);
    assert_eq!(pubs[0].payload, b"offline");
    assert!(pubs[0].retain);
}

#[tokio::test(start_paused = true)]
async fn subscribe_to_commands_subscribes_both_topics() {
    let (conn, _rx, client_handle) = setup_connected_session(vec![connack_step()]).await;
    conn.subscribe_to_commands().unwrap();
    tokio::task::yield_now().await;

    let subs = client_handle.subscribes();
    assert_eq!(subs.len(), 2);
    let topics: Vec<&str> = subs.iter().map(|s| s.topic.as_str()).collect();
    assert!(
        topics.contains(&"testbms/charging/set"),
        "missing charging/set subscription"
    );
    assert!(
        topics.contains(&"testbms/balancing/set"),
        "missing balancing/set subscription"
    );
    assert!(subs.iter().all(|s| s.qos == rumqttc::QoS::AtLeastOnce));
}

#[tokio::test(start_paused = true)]
async fn publish_discovery_all_topics_use_retain() {
    use crate::jkbms::JkBmsDeviceInfo;
    let device_info = JkBmsDeviceInfo {
        model: "Test".into(),
        hardware_version: "1.0".into(),
        software_version: "1.0".into(),
        serial_number: "T001".into(),
        power_cycle_count: 1,
    };
    let (conn, _rx, client_handle) = setup_connected_session(vec![connack_step()]).await;
    conn.publish_discovery(&device_info, 8).unwrap();
    tokio::task::yield_now().await;

    let pubs = client_handle.publishes();
    assert!(
        !pubs.is_empty(),
        "discovery must publish at least one topic"
    );
    assert!(
        pubs.iter().all(|p| p.retain),
        "all discovery topics must be retained"
    );
}

#[tokio::test(start_paused = true)]
async fn publish_snapshot_includes_state_topic_without_retain() {
    use crate::domain::Snapshot;
    let snapshot = Snapshot {
        cell_voltages_v: vec![3.3; 8],
        cell_resistances_ohm: vec![0.05; 8],
        ..Snapshot::default()
    };
    let (conn, _rx, client_handle) = setup_connected_session(vec![connack_step()]).await;
    conn.publish_snapshot(&snapshot).unwrap();
    tokio::task::yield_now().await;

    let pubs = client_handle.publishes();
    assert!(!pubs.is_empty());
    let state = pubs.iter().find(|p| p.topic == "testbms/state");
    assert!(state.is_some(), "testbms/state topic must be published");
    assert!(!state.unwrap().retain, "state topic must NOT be retained");
    assert!(
        pubs.iter().all(|p| !p.retain),
        "snapshot publishes must not be retained"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// Group 3 — Backoff scheduling
// ═════════════════════════════════════════════════════════════════════════════

/// Consecutive unstable sessions (no ConnAck) → backoff doubles each time.
///
/// The backoff variable starts at INITIAL_BACKOFF (2s) and is doubled BEFORE
/// each sleep, so the actual sleep durations are 2×, 4×, 8× INITIAL_BACKOFF.
#[tokio::test(start_paused = true)]
async fn backoff_doubles_on_unstable_sessions() {
    init_tracing();
    let mut sessions = Vec::new();
    for _ in 0..3 {
        let (c, el, _) = make_session(vec![io_error_step()]);
        sessions.push((c, el));
    }
    let (c_last, el_last, _) = make_session(vec![]); // parks until manager exits
    sessions.push((c_last, el_last));

    let (factory, fh) = MqttClientFactoryMock::new(sessions);
    let (_cmd_tx, _events_rx) = new_manager(test_config(), Box::new(factory));

    // Session 0 starts at t=0 and fails immediately; backoff doubles to 2×IB.
    tokio::task::yield_now().await;

    // Advance past 2×INITIAL_BACKOFF → session 1 starts.
    tokio::time::advance(INITIAL_BACKOFF * 2 + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    // Advance past 4×INITIAL_BACKOFF → session 2 starts.
    tokio::time::advance(INITIAL_BACKOFF * 4 + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    // Advance past 8×INITIAL_BACKOFF → session 3 starts.
    tokio::time::advance(INITIAL_BACKOFF * 8 + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    let ts = fh.session_timestamps();
    assert!(
        ts.len() >= 4,
        "expected at least 4 sessions, got {}",
        ts.len()
    );

    let gap1 = ts[1] - ts[0]; // should be 2×IB = 4s
    let gap2 = ts[2] - ts[1]; // should be 4×IB = 8s
    let gap3 = ts[3] - ts[2]; // should be 8×IB = 16s

    assert!(
        gap1 >= INITIAL_BACKOFF * 2 && gap1 < INITIAL_BACKOFF * 4,
        "gap[0→1] expected ~{:?}, got {gap1:?}",
        INITIAL_BACKOFF * 2
    );
    assert!(
        gap2 >= INITIAL_BACKOFF * 4 && gap2 < INITIAL_BACKOFF * 8,
        "gap[1→2] expected ~{:?}, got {gap2:?}",
        INITIAL_BACKOFF * 4
    );
    assert!(
        gap3 >= INITIAL_BACKOFF * 8 && gap3 < INITIAL_BACKOFF * 16,
        "gap[2→3] expected ~{:?}, got {gap3:?}",
        INITIAL_BACKOFF * 8
    );
}

/// Backoff never exceeds MAX_BACKOFF regardless of how many failures occur.
#[tokio::test(start_paused = true)]
async fn backoff_caps_at_max() {
    init_tracing();
    let n_fail: u32 = 8;
    let mut sessions = Vec::new();
    for _ in 0..n_fail {
        let (c, el, _) = make_session(vec![io_error_step()]);
        sessions.push((c, el));
    }
    let (c_last, el_last, _) = make_session(vec![]);
    sessions.push((c_last, el_last));

    let (factory, fh) = MqttClientFactoryMock::new(sessions);
    let (_cmd_tx, _events_rx) = new_manager(test_config(), Box::new(factory));

    tokio::task::yield_now().await;
    for _ in 0..n_fail {
        tokio::time::advance(MAX_BACKOFF + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;
    }

    let ts = fh.session_timestamps();
    for i in 1..ts.len() {
        let gap = ts[i] - ts[i - 1];
        assert!(
            gap <= MAX_BACKOFF + Duration::from_millis(2),
            "gap[{}→{}] = {gap:?} exceeds MAX_BACKOFF={MAX_BACKOFF:?}",
            i - 1,
            i
        );
    }
}

/// A session that stayed connected >= STABLE_THRESHOLD resets backoff to
/// INITIAL_BACKOFF regardless of prior doubling.
///
/// Sessions:
///   0: io_error immediately → backoff = 2×IB (4s)
///   1: ConnAck + delay(STABLE_THRESHOLD+ε) + io_error → stable, backoff resets to IB (2s)
///   2: io_error immediately → backoff = 2×IB (4s) — key assertion: NOT 4×IB (8s)
///   3: parks forever
///
/// If the reset had not happened (backoff still at 2×IB=4s from session 0),
/// session 2's sleep would be 4×IB=8s instead of 2×IB=4s.
#[tokio::test(start_paused = true)]
async fn backoff_resets_after_stable_session() {
    init_tracing();
    // Session 0: fails immediately, inflates backoff to 2×IB.
    let (c0, el0, _) = make_session(vec![io_error_step()]);
    // Session 1: stable (ConnAck + long delay + error).
    let (c1, el1, _) = make_session(vec![
        connack_step(),
        delay_step(STABLE_THRESHOLD + Duration::from_millis(1)),
        io_error_step(),
    ]);
    // Session 2: fails immediately; gap should be 2×IB (reset), not 4×IB (doubled again).
    let (c2, el2, _) = make_session(vec![io_error_step()]);
    // Session 3: fails immediately to let the test observe the doubled gap.
    let (c3, el3, _) = make_session(vec![io_error_step()]);
    // Session 4: parks to prevent factory exhaustion.
    let (c4, el4, _) = make_session(vec![]);

    let (factory, fh) =
        MqttClientFactoryMock::new(vec![(c0, el0), (c1, el1), (c2, el2), (c3, el3), (c4, el4)]);
    let (_cmd_tx, mut events_rx) = new_manager(test_config(), Box::new(factory));

    // Session 0 starts at t=0, fails immediately; sleep(2×IB=4s) begins.
    tokio::task::yield_now().await;

    // Advance past 2×IB → session 1 starts.
    tokio::time::advance(INITIAL_BACKOFF * 2 + Duration::from_millis(1)).await;
    recv_match(&mut events_rx, is_broker_connected, "BrokerConnected").await;

    // Session 1 is sleeping on delay_step(STABLE_THRESHOLD+1ms). Advance to
    // complete it → io_error fires → session ends as stable.
    tokio::time::advance(STABLE_THRESHOLD + Duration::from_millis(2)).await;
    recv_match(&mut events_rx, is_broker_disconnected, "BrokerDisconnected").await;

    // Backoff reset to IB (2s). Advance IB+ε → session 2 starts.
    tokio::time::advance(INITIAL_BACKOFF + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    // Session 2 fails; backoff doubles to 2×IB (4s). Advance 2×IB+ε → session 3.
    tokio::time::advance(INITIAL_BACKOFF * 2 + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    // Session 3 fails; backoff doubles to 4×IB (8s). Advance 4×IB+ε → session 4.
    tokio::time::advance(INITIAL_BACKOFF * 4 + Duration::from_millis(1)).await;
    tokio::task::yield_now().await;

    let ts = fh.session_timestamps();
    assert!(ts.len() >= 5, "expected ≥ 5 sessions, got {}", ts.len());

    // gap[2→3]: first failure AFTER reset. backoff = IB → doubled to 2×IB → sleep(2×IB).
    // Sessions fail immediately so gap ≈ sleep duration.
    let gap_2_3 = ts[3] - ts[2];
    assert!(
        gap_2_3 >= INITIAL_BACKOFF * 2 && gap_2_3 < INITIAL_BACKOFF * 4,
        "gap[2→3] (first failure after reset) should be 2×IB={:?}, got {gap_2_3:?}",
        INITIAL_BACKOFF * 2
    );

    // gap[3→4]: second consecutive failure after reset → doubles again to 4×IB.
    let gap_3_4 = ts[4] - ts[3];
    assert!(
        gap_3_4 >= INITIAL_BACKOFF * 4 && gap_3_4 < INITIAL_BACKOFF * 8,
        "gap[3→4] (second failure after reset) should be 4×IB={:?}, got {gap_3_4:?}",
        INITIAL_BACKOFF * 4
    );
}
