use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};

use crate::coordinator::coordinator::Coordinator;
use crate::healthcheck::{HealthQuery, HealthStatus};
use crate::jkbms::{
    ConnectionState, JkBmsConfigOptions, JkBmsData, JkBmsDeviceInfo, JkBmsEvents,
    JkBmsOperationalData, WriteCommand,
};
use crate::mqtt::tests::support::connection_mock::MqttConnectionMock;
use crate::mqtt::{IncomingRequest, MqttEvents};

use super::support::connection_mock::JkBmsConnectionMock;

// ── Dummy data constructors ───────────────────────────────────────────────────

fn dummy_device_info() -> JkBmsDeviceInfo {
    JkBmsDeviceInfo {
        model: "JK_PB2A16S20P".into(),
        hardware_version: "15A".into(),
        software_version: "15.41".into(),
        serial_number: "REDACTED".into(),
        power_cycle_count: 39,
    }
}

fn dummy_config() -> JkBmsConfigOptions {
    JkBmsConfigOptions {
        cell_count: 8,
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
        total_voltage_v: 26.4,
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

// ── Test harness ──────────────────────────────────────────────────────────────

struct Harness {
    mqtt: Arc<MqttConnectionMock>,
    jkbms: Arc<JkBmsConnectionMock>,
    mqtt_events_tx: mpsc::UnboundedSender<MqttEvents>,
    jkbms_events_tx: mpsc::UnboundedSender<JkBmsEvents>,
    health_query_tx: mpsc::Sender<HealthQuery>,
}

impl Harness {
    fn new() -> Self {
        let (mqtt_mock, mqtt_events_rx) = MqttConnectionMock::new();
        let mqtt_events_tx = mqtt_mock.events_tx.clone();
        let mqtt = Arc::new(mqtt_mock);

        let jkbms = Arc::new(JkBmsConnectionMock::new());
        let (jkbms_events_tx, jkbms_events_rx) = mpsc::unbounded_channel();

        let (health_query_tx, health_query_rx) = mpsc::channel::<HealthQuery>(4);

        let coordinator = Coordinator::new(
            Box::new(mqtt.clone()),
            Box::new(jkbms.clone()),
            jkbms_events_rx,
            mqtt_events_rx,
            Some(health_query_rx),
        );
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        tokio::spawn(coordinator.run(shutdown_rx));

        Harness {
            mqtt,
            jkbms,
            mqtt_events_tx,
            jkbms_events_tx,
            health_query_tx,
        }
    }

    fn send_jkbms_event(&self, event: JkBmsEvents) {
        self.jkbms_events_tx.send(event).unwrap();
    }

    async fn send_jkbms_event_and_wait(&self, event: JkBmsEvents) {
        self.jkbms_events_tx.send(event).unwrap();
        self.yield_n(4).await;
    }

    fn send_mqtt_event(&self, event: MqttEvents) {
        self.mqtt_events_tx.send(event).unwrap();
    }

    async fn send_mqtt_event_and_wait(&self, event: MqttEvents) {
        self.mqtt_events_tx.send(event).unwrap();
        self.yield_n(4).await;
    }

    async fn yield_n(&self, n: usize) {
        for _ in 0..n {
            tokio::task::yield_now().await;
        }
    }

    fn availability_strings(&self) -> Vec<&'static str> {
        self.mqtt
            .availability_calls()
            .into_iter()
            .map(|online| if online { "online" } else { "offline" })
            .collect()
    }

    async fn query_health(&self) -> HealthStatus {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.health_query_tx
            .send(HealthQuery::Get(reply_tx))
            .await
            .unwrap();
        reply_rx.await.unwrap()
    }
}

async fn full_startup(h: &Harness) {
    h.send_jkbms_event(JkBmsEvents::Data(
        JkBmsData::DeviceInfo(dummy_device_info()),
    ));
    h.send_jkbms_event(JkBmsEvents::Data(JkBmsData::ConfigOptions(dummy_config())));
    h.send_jkbms_event_and_wait(JkBmsEvents::Data(JkBmsData::OperationalData(Box::new(
        dummy_operational(),
    ))))
    .await;
    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Connected))
        .await;
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test(start_paused = true)]
async fn operational_data_publishes_all_sensors() {
    let h = Harness::new();
    full_startup(&h).await;

    let snap = h.mqtt.last_snapshot().expect("snapshot must be published");
    assert_eq!(format!("{:.2}", snap.total_voltage_v), "26.40");
    assert_eq!(format!("{:.3}", snap.cell_voltages_v[0]), "3.300");
    assert_eq!(snap.soc_pct, 80);
    assert_eq!(format!("{:.1}", snap.mos_temperature_c), "25.0");
    assert!(snap.charging_switch);
    assert!(!snap.balancing_active);

    // Availability transitioned to online
    let avail = h.availability_strings();
    assert!(
        avail.contains(&"online"),
        "expected availability=online, got: {avail:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn initial_discovery_published_after_first_operational_data() {
    let h = Harness::new();

    // DeviceInfo + ConfigOptions alone must NOT trigger discovery: the gate
    // requires has_operational too, so HA never sees entities before a first snapshot
    // can be published.
    h.send_jkbms_event(JkBmsEvents::Data(
        JkBmsData::DeviceInfo(dummy_device_info()),
    ));
    h.send_jkbms_event_and_wait(JkBmsEvents::Data(JkBmsData::ConfigOptions(dummy_config())))
        .await;

    assert!(
        h.mqtt.discovery_cell_counts().is_empty(),
        "discovery must not be published before first operational data"
    );

    // Operational data arrives → aggregator has all three → discovery published.
    h.send_jkbms_event_and_wait(JkBmsEvents::Data(JkBmsData::OperationalData(Box::new(
        dummy_operational(),
    ))))
    .await;

    let discoveries = h.mqtt.discovery_cell_counts();
    assert!(
        !discoveries.is_empty(),
        "expected discovery to be published after first operational data"
    );
    assert_eq!(
        *discoveries.last().unwrap(),
        8u32,
        "expected cell_count=8 in discovery"
    );

    // Discovery published exactly once — re-sending events must not republish.
    let count_before = discoveries.len();
    h.send_jkbms_event_and_wait(JkBmsEvents::Data(
        JkBmsData::DeviceInfo(dummy_device_info()),
    ))
    .await;
    let count_after = h.mqtt.discovery_cell_counts().len();
    assert_eq!(
        count_before, count_after,
        "discovery must not republish on repeated DeviceInfo"
    );
}

#[tokio::test(start_paused = true)]
async fn set_charging_dispatches_jk_write_and_freezes() {
    let h = Harness::new();

    h.send_mqtt_event_and_wait(MqttEvents::Incoming(IncomingRequest::SetCharging(true)))
        .await;

    let writes = h.jkbms.writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0], (WriteCommand::SetCharging(true), 1));
}

#[tokio::test(start_paused = true)]
async fn write_confirmation_publishes_switch_state() {
    let h = Harness::new();
    full_startup(&h).await;

    let mut confirmed_cfg = dummy_config();
    confirmed_cfg.charging_switch = true;
    h.send_jkbms_event_and_wait(JkBmsEvents::WriteConfirmation {
        seq: 1,
        data: Box::new(confirmed_cfg),
    })
    .await;

    let snap = h.mqtt.last_snapshot().expect("snapshot must be published");
    assert!(snap.charging_switch, "charging must be ON");
}

#[tokio::test(start_paused = true)]
async fn write_confirmation_ignored_when_superseded() {
    let h = Harness::new();
    full_startup(&h).await;

    // Freeze seq=2 (the newer pending write; seq=1 confirmation is stale)
    h.send_mqtt_event(MqttEvents::Incoming(IncomingRequest::SetCharging(false))); // seq=1
    h.send_mqtt_event_and_wait(MqttEvents::Incoming(IncomingRequest::SetCharging(true))) // seq=2
        .await;

    // Confirm seq=1 with charging=false — freeze for seq=2 must survive
    let mut stale_cfg = dummy_config();
    stale_cfg.charging_switch = false;
    h.send_jkbms_event_and_wait(JkBmsEvents::WriteConfirmation {
        seq: 1,
        data: Box::new(stale_cfg),
    })
    .await;

    // The most recent snapshot must reflect the seq=2 freeze (charging=true)
    let snap = h.mqtt.last_snapshot().expect("snapshot must be published");
    assert!(
        snap.charging_switch,
        "seq=2 freeze (true) must override stale seq=1 confirmation"
    );
}

#[tokio::test(start_paused = true)]
async fn write_error_clears_freeze() {
    let h = Harness::new();
    full_startup(&h).await;

    // Request SetCharging(false) — coordinator freezes false for seq=1
    h.send_mqtt_event_and_wait(MqttEvents::Incoming(IncomingRequest::SetCharging(false)))
        .await;

    // OperationalData arrives before the error: BMS still reports charging=true (write
    // not applied yet). The rebuilt snapshot has the freeze applied, so the cached
    // `last_snapshot` now holds charging=false — the same value the write asked for.
    h.send_jkbms_event_and_wait(JkBmsEvents::Data(JkBmsData::OperationalData(Box::new(
        dummy_operational(),
    ))))
    .await;
    assert!(
        !h.mqtt
            .last_snapshot()
            .expect("snapshot must be published")
            .charging_switch,
        "freeze must override BMS value while the write is pending"
    );

    // Send error for seq=1. Freeze clears; the coordinator must rebuild from the
    // aggregator (not republish the stale, still-frozen cache) so HA sees the revert.
    h.send_jkbms_event_and_wait(JkBmsEvents::WriteError { seq: 1 })
        .await;

    let snap = h.mqtt.last_snapshot().expect("snapshot must be published");
    assert!(
        snap.charging_switch,
        "after error freeze cleared; BMS actual value (ON) should be published"
    );
}

#[tokio::test(start_paused = true)]
async fn broker_reconnect_reflects_pending_freeze() {
    let h = Harness::new();
    full_startup(&h).await;

    // HA request lands but no OperationalData or write outcome yet — freeze is the
    // only place the intent lives.
    h.send_mqtt_event_and_wait(MqttEvents::Incoming(IncomingRequest::SetCharging(false)))
        .await;

    h.send_mqtt_event_and_wait(MqttEvents::BrokerConnected)
        .await;

    let snap = h.mqtt.last_snapshot().expect("snapshot must be published");
    assert!(
        !snap.charging_switch,
        "broker reconnect must republish the current (frozen) state, not a stale pre-request snapshot"
    );
}

#[tokio::test(start_paused = true)]
async fn mqtt_reconnect_republishes_discovery_and_snapshot() {
    let h = Harness::new();
    full_startup(&h).await;

    let snap_count_before = h.mqtt.snapshots().len();
    let disc_count_before = h.mqtt.discovery_cell_counts().len();
    let avail_count_before = h.mqtt.availability_calls().len();

    h.send_mqtt_event_and_wait(MqttEvents::BrokerConnected)
        .await;

    // subscribe_to_commands must have been called
    assert!(
        h.mqtt.subscribe_commands_count() > 0,
        "subscribe_to_commands must be called on broker connect"
    );

    // Discovery republished
    assert!(
        h.mqtt.discovery_cell_counts().len() > disc_count_before,
        "discovery must be republished on broker connect"
    );

    // Availability republished
    assert!(
        h.mqtt.availability_calls().len() > avail_count_before,
        "availability must be republished on broker connect"
    );

    // Snapshot republished
    assert!(
        h.mqtt.snapshots().len() > snap_count_before,
        "snapshot must be republished on broker connect"
    );
}

#[tokio::test(start_paused = true)]
async fn reconnecting_does_not_flip_availability_offline() {
    let h = Harness::new();
    full_startup(&h).await;

    // Sanity: startup put us online.
    let avail_before = h.availability_strings();
    assert!(avail_before.contains(&"online"));
    let offline_before = avail_before.iter().filter(|&&p| p == "offline").count();

    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Reconnecting))
        .await;

    let avail_after = h.availability_strings();
    let offline_after = avail_after.iter().filter(|&&p| p == "offline").count();
    assert_eq!(
        offline_after, offline_before,
        "Reconnecting must not flip availability offline; got: {avail_after:?}"
    );
    assert_eq!(
        h.query_health().await,
        HealthStatus::Unhealthy,
        "broker was never connected so health stays Unhealthy regardless of Reconnecting",
    );
}

#[tokio::test(start_paused = true)]
async fn disconnected_flips_availability_offline() {
    let h = Harness::new();
    full_startup(&h).await;

    // Sanity: startup put us online.
    assert!(h.availability_strings().contains(&"online"));

    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Disconnected))
        .await;

    let avail = h.availability_strings();
    assert!(
        avail.contains(&"offline"),
        "Disconnected must publish availability=offline; got: {avail:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn broker_disconnected_marks_unhealthy() {
    let h = Harness::new();

    // Startup: broker connects and the BMS reports Connected so coordinator is healthy.
    h.send_mqtt_event_and_wait(MqttEvents::BrokerConnected)
        .await;
    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Connected))
        .await;

    assert_eq!(
        h.query_health().await,
        HealthStatus::Healthy,
        "should be healthy after Connected + broker connected"
    );

    // Broker disconnects → unhealthy
    h.send_mqtt_event_and_wait(MqttEvents::BrokerDisconnected)
        .await;
    assert_eq!(
        h.query_health().await,
        HealthStatus::Unhealthy,
        "should be unhealthy after broker disconnected"
    );
}

#[tokio::test(start_paused = true)]
async fn connection_events_drive_health() {
    let h = Harness::new();

    h.send_mqtt_event_and_wait(MqttEvents::BrokerConnected)
        .await;

    // Before any Connection event: unhealthy (jkbms_online still false).
    assert_eq!(
        h.query_health().await,
        HealthStatus::Unhealthy,
        "unhealthy before first Connected"
    );

    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Connected))
        .await;
    assert_eq!(
        h.query_health().await,
        HealthStatus::Healthy,
        "healthy after Connected"
    );

    // Reconnecting must NOT drop health.
    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Reconnecting))
        .await;
    assert_eq!(
        h.query_health().await,
        HealthStatus::Healthy,
        "Reconnecting is transient and must not drop health"
    );

    // Disconnected drops it.
    h.send_jkbms_event_and_wait(JkBmsEvents::Connection(ConnectionState::Disconnected))
        .await;
    assert_eq!(
        h.query_health().await,
        HealthStatus::Unhealthy,
        "unhealthy after Disconnected"
    );
}

#[tokio::test]
async fn all_connections_stopped_on_shutdown() {
    let (mqtt_mock, mqtt_events_rx) = MqttConnectionMock::new();
    let mqtt = Arc::new(mqtt_mock);

    let jkbms = Arc::new(JkBmsConnectionMock::new());
    let (_jkbms_events_tx, jkbms_events_rx) = mpsc::unbounded_channel::<JkBmsEvents>();

    let (_health_query_tx, health_query_rx) = mpsc::channel::<HealthQuery>(4);

    let coordinator = Coordinator::new(
        Box::new(mqtt.clone()),
        Box::new(jkbms.clone()),
        jkbms_events_rx,
        mqtt_events_rx,
        Some(health_query_rx),
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(coordinator.run(shutdown_rx));

    let _ = shutdown_tx.send(true);
    let _ = task.await;

    assert!(
        mqtt.stop_called(),
        "IMqttConnection::stop must be called on shutdown"
    );
    assert!(
        jkbms.stop_called(),
        "IJkBmsConnection::stop must be called on shutdown"
    );
}
