use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::domain::Snapshot;
use crate::healthcheck::{HealthQuery, HealthStatus};
use crate::jkbms::{ConnectionState, IJkBmsConnection, JkBmsData, JkBmsEvents, WriteCommand};
use crate::mqtt::{IMqttConnection, IncomingRequest, MqttEvents};

use super::aggregator::StateAggregator;
use super::freeze::{SwitchField, SwitchFreeze};

pub struct Coordinator {
    mqtt: Box<dyn IMqttConnection>,
    jkbms: Box<dyn IJkBmsConnection>,
    aggregator: StateAggregator,
    freeze: SwitchFreeze,
    next_seq: u64,
    jkbms_reconnect_count: u32,
    had_first_connection: bool,
    mqtt_reconnect_count: u32,
    had_first_broker_connection: bool,
    discovery_published: bool,

    broker_connected: bool,
    jkbms_online: bool,

    jkbms_events_rx: mpsc::UnboundedReceiver<JkBmsEvents>,
    mqtt_events_rx: mpsc::UnboundedReceiver<MqttEvents>,
    health_query_rx: Option<mpsc::Receiver<HealthQuery>>,
}

impl Coordinator {
    pub fn new(
        mqtt: Box<dyn IMqttConnection>,
        jkbms: Box<dyn IJkBmsConnection>,
        jkbms_events_rx: mpsc::UnboundedReceiver<JkBmsEvents>,
        mqtt_events_rx: mpsc::UnboundedReceiver<MqttEvents>,
        health_query_rx: Option<mpsc::Receiver<HealthQuery>>,
    ) -> Self {
        Self {
            mqtt,
            jkbms,
            aggregator: StateAggregator::new(),
            freeze: SwitchFreeze::new(),
            next_seq: 0,
            jkbms_reconnect_count: 0,
            had_first_connection: false,
            mqtt_reconnect_count: 0,
            had_first_broker_connection: false,
            discovery_published: false,
            broker_connected: false,
            jkbms_online: false,
            jkbms_events_rx,
            mqtt_events_rx,
            health_query_rx,
        }
    }

    pub async fn run(mut self, mut shutdown_rx: watch::Receiver<bool>) {
        loop {
            tokio::select! {
                event = self.mqtt_events_rx.recv() => {
                    match event {
                        Some(MqttEvents::BrokerConnected) => {
                            self.broker_connected = true;
                            if self.had_first_broker_connection {
                                self.mqtt_reconnect_count =
                                    self.mqtt_reconnect_count.saturating_add(1);
                            } else {
                                self.had_first_broker_connection = true;
                            }
                            self.handle_broker_connected().await;
                        }
                        Some(MqttEvents::BrokerDisconnected) => {
                            self.broker_connected = false;
                        }
                        Some(MqttEvents::Incoming(req)) => self.handle_incoming(req).await,
                        None => {
                            tracing::warn!("MQTT event channel closed; exiting coordinator");
                            break;
                        }
                    }
                }
                event = self.jkbms_events_rx.recv() => {
                    match event {
                        Some(event) => self.handle_jkbms_event(event).await,
                        None => {
                            tracing::warn!("JKBMS event channel closed; exiting coordinator");
                            break;
                        }
                    }
                }
                Some(HealthQuery::Get(reply_tx)) = async {
                    match self.health_query_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    let _ = reply_tx.send(self.health_status());
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!("coordinator shutting down");
                        break;
                    }
                }
            }
        }
        // Gracefully stop nested tasks
        let mqtt_task = self.mqtt.stop();
        let jkbms_task = self.jkbms.stop();
        let _ = mqtt_task.await;
        let _ = jkbms_task.await;
    }

    fn next_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn health_status(&self) -> HealthStatus {
        if self.broker_connected && self.jkbms_online {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        }
    }

    async fn handle_incoming(&mut self, req: IncomingRequest) {
        match req {
            IncomingRequest::SetCharging(value) => {
                let seq = self.next_seq();
                let command = WriteCommand::SetCharging(value);
                tracing::info!(seq, command = ?command, "write command received");
                self.freeze.freeze(seq, SwitchField::Charging, value);
                if self.jkbms.write(command, seq).is_err() {
                    self.freeze.clear(seq);
                    tracing::warn!(seq, "JKBMS command channel full; dropping SetCharging");
                }
            }
            IncomingRequest::SetBalancing(value) => {
                let seq = self.next_seq();
                let command = WriteCommand::SetBalancing(value);
                tracing::info!(seq, command = ?command, "write command received");
                self.freeze.freeze(seq, SwitchField::Balancing, value);
                if self.jkbms.write(command, seq).is_err() {
                    self.freeze.clear(seq);
                    tracing::warn!(seq, "JKBMS command channel full; dropping SetBalancing");
                }
            }
        }
    }

    async fn handle_jkbms_event(&mut self, event: JkBmsEvents) {
        let had_device_info = self.aggregator.has_device_info();
        let had_config = self.aggregator.has_config_options();
        let had_operational = self.aggregator.has_operational();

        match event {
            // Events that don't feed the tail (no aggregator/freeze state changes).
            JkBmsEvents::Connection(state) => {
                match state {
                    ConnectionState::Connected => {
                        if self.had_first_connection {
                            self.jkbms_reconnect_count =
                                self.jkbms_reconnect_count.saturating_add(1);
                        } else {
                            self.had_first_connection = true;
                        }
                        tracing::info!("JKBMS connection established");
                        self.jkbms_online = true;
                        self.publish_availability(true);
                    }
                    ConnectionState::Reconnecting => {
                        // Transient link loss inside the fast-reconnect window —
                        // deliberately do not flip availability offline.
                        tracing::info!("JKBMS connection is temporary lost, reconnecting");
                    }
                    ConnectionState::Disconnected => {
                        if self.jkbms_online {
                            tracing::warn!("JKBMS connnection lost, will reconnect once possibe");
                            self.jkbms_online = false;
                            self.publish_availability(false);
                        }
                    }
                }
                return;
            }

            // Events that mutate aggregator or freeze state and fall through to the
            // discovery + snapshot publish tail below.
            JkBmsEvents::Data(data) => match data {
                JkBmsData::DeviceInfo(info) => self.aggregator.set_device_info(info),
                JkBmsData::ConfigOptions(opts) => self.aggregator.set_config_options(opts),
                JkBmsData::OperationalData(op) => {
                    tracing::debug!("operational data received");
                    self.aggregator.set_operational(*op, Instant::now());
                }
                JkBmsData::Alarms(raw) => self.aggregator.set_alarms(raw),
            },
            JkBmsEvents::WriteConfirmation { seq, data } => {
                self.freeze.apply(seq);
                self.aggregator.set_config_options(*data);
            }
            JkBmsEvents::WriteError { seq } => {
                self.freeze.clear(seq);
            }
        }

        // Runs for any event that mutated aggregator or freeze state
        // (Data / WriteConfirmation / WriteError).
        // Trigger discovery on the first time DeviceInfo + ConfigOptions + OperationalData are all known.
        if !self.discovery_published
            && self.aggregator.has_device_info()
            && self.aggregator.has_config_options()
            && self.aggregator.has_operational()
            && (!had_device_info || !had_config || !had_operational)
        {
            self.do_publish_discovery();
            self.discovery_published = true;
        }

        // Publish snapshot whenever we have enough data.
        if let Some(snap) = self.build_snapshot() {
            self.do_publish_snapshot(&snap);
        } else {
            tracing::debug!(
                has_device_info = self.aggregator.has_device_info(),
                has_config = self.aggregator.has_config_options(),
                has_operational = self.aggregator.has_operational(),
                "snapshot not yet buildable"
            );
        }
    }

    async fn handle_broker_connected(&mut self) {
        tracing::info!("MQTT broker connected");
        if self.mqtt.subscribe_to_commands().is_err() {
            tracing::warn!("failed to enqueue subscribe (command channel full)");
            return;
        }

        // Publish availability before discovery so HA sees the correct retained
        // state when it subscribes to the availability topic upon processing discovery.
        if self.mqtt.publish_availability(self.jkbms_online).is_err() {
            tracing::warn!("failed to enqueue availability republish (command channel full)");
        }

        // Republish discovery if we have both DeviceInfo and ConfigOptions.
        let discovery_data = self
            .aggregator
            .device_info()
            .cloned()
            .zip(self.aggregator.config_options().map(|c| c.cell_count));
        if let Some((info, cell_count)) = discovery_data {
            if self.mqtt.publish_discovery(&info, cell_count).is_err() {
                tracing::warn!("failed to enqueue discovery republish (command channel full)");
            }
        }

        if let Some(snap) = self.build_snapshot() {
            self.do_publish_snapshot(&snap);
        }
    }

    fn build_snapshot(&self) -> Option<Snapshot> {
        let mut snap = self.aggregator.snapshot()?;
        self.freeze.apply_to(&mut snap);
        snap.jkbms_reconnect_count = self.jkbms_reconnect_count;
        snap.mqtt_reconnect_count = self.mqtt_reconnect_count;
        Some(snap)
    }

    fn do_publish_snapshot(&self, snap: &Snapshot) {
        tracing::debug!("publishing snapshot");
        if self.mqtt.publish_snapshot(snap).is_err() {
            tracing::warn!("failed to enqueue snapshot (command channel full)");
        }
    }

    fn do_publish_discovery(&mut self) {
        let info = match self.aggregator.device_info() {
            Some(i) => i.clone(),
            None => return,
        };
        let cell_count = match self.aggregator.config_options() {
            Some(c) => c.cell_count,
            None => return,
        };
        tracing::info!("publishing discovery");
        if self.mqtt.publish_discovery(&info, cell_count).is_err() {
            tracing::warn!("failed to enqueue discovery (command channel full)");
        }
    }

    fn publish_availability(&self, online: bool) {
        tracing::info!(online, "publishing availability");
        if self.mqtt.publish_availability(online).is_err() {
            tracing::warn!("failed to enqueue availability (command channel full)");
        }
    }
}
