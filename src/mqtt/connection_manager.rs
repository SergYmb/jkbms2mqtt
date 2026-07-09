//! Actor owning the `(AsyncClient, EventLoop)` pair. Follows rumqttc's
//! canonical two-task pattern (`examples/asyncpubsub.rs`): the eventloop
//! runs in a spawned tokio task, dispatch runs in the main task. Shutdown
//! is signalled by a watch channel (`false` → `true`); the eventloop checks
//! it on every `select!` iteration so it exits without waiting for in-flight
//! network I/O to complete.

use std::time::Duration;

use rumqttc::{ClientError, Event, LastWill, MqttOptions, Packet, QoS};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::domain::Snapshot;
use crate::jkbms::JkBmsDeviceInfo;

use super::config::MqttConfig;
use super::events::MqttEvents;
use super::mqttc_wrapper::{IMqttClient, IMqttClientFactory, IMqttEventLoop};
use super::{discovery, formatter, inbound, topics};

const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const REQUEST_CHANNEL_CAPACITY: usize = 32;
const KEEP_ALIVE: Duration = Duration::from_secs(30);
const STABLE_THRESHOLD: Duration = MAX_BACKOFF;
const FAST_FAIL_WINDOW: Duration = Duration::from_secs(5);

pub(super) enum MqttCommand {
    PublishSnapshot(Box<Snapshot>),
    PublishDiscovery {
        device_info: Box<JkBmsDeviceInfo>,
        cell_count: u32,
    },
    PublishAvailability(bool),
    SubscribeToCommands,
}

enum SessionOutcome {
    HandleDropped,
    Disconnected,
}

pub(super) struct MqttConnectionManager {
    config: MqttConfig,
    commands_rx: mpsc::Receiver<MqttCommand>,
    events_tx: mpsc::UnboundedSender<MqttEvents>,
    session_id: u64,
    client_factory: Box<dyn IMqttClientFactory>,
}

impl MqttConnectionManager {
    pub(super) fn new(
        config: MqttConfig,
        commands_rx: mpsc::Receiver<MqttCommand>,
        events_tx: mpsc::UnboundedSender<MqttEvents>,
        client_factory: Box<dyn IMqttClientFactory>,
    ) -> Self {
        Self {
            config,
            commands_rx,
            events_tx,
            session_id: 0,
            client_factory,
        }
    }

    pub(super) async fn run(mut self) {
        let mut backoff = INITIAL_BACKOFF;
        loop {
            let (outcome, session_was_stable) = self.run_new_session().await;
            match outcome {
                SessionOutcome::HandleDropped => break,
                SessionOutcome::Disconnected => {
                    backoff = if session_was_stable {
                        INITIAL_BACKOFF
                    } else {
                        (backoff * 2).min(MAX_BACKOFF)
                    };
                    tokio::time::sleep(backoff).await;
                }
            }
        }
        tracing::info!("MqttConnectionManager run loop finished");
    }

    async fn run_new_session(&mut self) -> (SessionOutcome, bool) {
        self.session_id += 1;
        let opts = build_mqtt_options(&self.config);
        let (client, eventloop) = self
            .client_factory
            .new_client(opts, REQUEST_CHANNEL_CAPACITY);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let mut poll_handle = tokio::spawn(run_eventloop(
            eventloop,
            self.events_tx.clone(),
            self.session_id,
            shutdown_rx,
        ));

        // listen commands and react on eventloop exit
        let (dispatch_outcome, poll_result) = tokio::select! {
            outcome = async {
                loop {
                    match self.commands_rx.recv().await {
                        None => return SessionOutcome::HandleDropped,
                        Some(cmd) => {
                            if let Err(e) = dispatch(client.as_ref(), cmd, &self.config).await {
                                tracing::warn!(error = %e, "MQTT dispatch failed");
                                return SessionOutcome::Disconnected;
                            }
                        }
                    }
                }
            } => {
                drop(client);
                let _ = shutdown_tx.send(true); // tell run_eventloop to exit its select! loop
                (Some(outcome), (&mut poll_handle).await)
            }
            // if eventloop exited first
            result = &mut poll_handle => (None, result),
        };

        let (poll_outcome, connected_at) = poll_result.expect("MQTT poll task panicked");
        let outcome = dispatch_outcome.unwrap_or(poll_outcome);

        if connected_at.is_some() && matches!(outcome, SessionOutcome::Disconnected) {
            let _ = self.events_tx.send(MqttEvents::BrokerDisconnected);
        }

        (
            outcome,
            connected_at.is_some_and(|t| t.elapsed() >= STABLE_THRESHOLD),
        )
    }
}

async fn run_eventloop(
    mut eventloop: Box<dyn IMqttEventLoop>,
    events_tx: mpsc::UnboundedSender<MqttEvents>,
    session_id: u64,
    mut shutdown_rx: watch::Receiver<bool>,
) -> (SessionOutcome, Option<Instant>) {
    let mut connected_at: Option<Instant> = None;
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    return (SessionOutcome::HandleDropped, connected_at);
                }
            }
            result = eventloop.poll() => match result {
                Ok(Event::Incoming(Packet::ConnAck(ca))) => {
                    tracing::info!(
                        session_id,
                        session_present = ca.session_present,
                        "MQTT broker connection established"
                    );
                    connected_at = Some(Instant::now());
                    let _ = events_tx.send(MqttEvents::BrokerConnected);
                }
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    tracing::trace!(topic = %p.topic, "mqtt receive");
                    if let Some(req) = inbound::parse_inbound_command(&p.topic, &p.payload) {
                        if events_tx.send(MqttEvents::Incoming(req)).is_err() {
                            return (SessionOutcome::HandleDropped, connected_at);
                        }
                    }
                }
                Ok(Event::Incoming(Packet::Disconnect)) => {
                    tracing::info!(session_id, "MQTT broker sent Disconnect");
                    return (SessionOutcome::Disconnected, connected_at);
                }
                Ok(_) => {}
                Err(e) => {
                    if connected_at.is_some_and(|t| t.elapsed() < FAST_FAIL_WINDOW) {
                        tracing::warn!(
                            session_id,
                            error = %e,
                            "MQTT poll failed shortly after connect (likely session takeover)"
                        );
                    } else {
                        tracing::info!(
                            session_id,
                            error = %e,
                            "MQTT poll failed"
                        );
                    }
                    return (SessionOutcome::Disconnected, connected_at);
                }
            },
        }
    }
}

async fn dispatch(
    client: &dyn IMqttClient,
    cmd: MqttCommand,
    config: &MqttConfig,
) -> Result<(), ClientError> {
    match cmd {
        MqttCommand::PublishSnapshot(snapshot) => {
            let pubs = formatter::per_entity_publications(&snapshot, &config.bms_name);
            tracing::debug!(n_topics = pubs.len() + 1, "publishing snapshot");
            for (topic, payload) in pubs {
                tracing::trace!(
                    topic = %topic,
                    payload = %String::from_utf8_lossy(&payload),
                    "mqtt publish"
                );
                client
                    .publish(&topic, QoS::AtLeastOnce, false, payload)
                    .await?;
            }
            let json = formatter::snapshot_json(&snapshot);
            let topic = format!("{}/state", config.bms_name);
            tracing::trace!(
                topic = %topic,
                payload = %String::from_utf8_lossy(&json),
                "mqtt publish"
            );
            client
                .publish(&topic, QoS::AtLeastOnce, false, json)
                .await?;
        }
        MqttCommand::PublishDiscovery {
            device_info,
            cell_count,
        } => {
            let payloads = discovery::build_payloads(
                &device_info,
                cell_count,
                &config.bms_name,
                &config.discovery_prefix,
            );
            tracing::debug!(n_topics = payloads.len(), "publishing discovery");
            for (topic, payload) in payloads {
                tracing::trace!(
                    topic = %topic,
                    payload = %String::from_utf8_lossy(&payload),
                    "mqtt publish"
                );
                client
                    .publish(&topic, QoS::AtLeastOnce, true, payload)
                    .await?;
            }
        }
        MqttCommand::PublishAvailability(online) => {
            let payload = if online {
                b"online".to_vec()
            } else {
                b"offline".to_vec()
            };
            let topic = topics::availability_topic(&config.bms_name);
            tracing::debug!(online, topic = %topic, "publishing availability");
            tracing::trace!(
                topic = %topic,
                payload = %String::from_utf8_lossy(&payload),
                "mqtt publish"
            );
            client
                .publish(&topic, QoS::AtLeastOnce, true, payload)
                .await?;
        }
        MqttCommand::SubscribeToCommands => {
            for cmd in inbound::inbound_commands() {
                client
                    .subscribe(&topics::set_topic(&config.bms_name, cmd), QoS::AtLeastOnce)
                    .await?;
            }
        }
    }
    Ok(())
}

fn build_mqtt_options(config: &MqttConfig) -> MqttOptions {
    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| format!("jkbms2mqtt-{}", config.bms_name));

    let mut opts = MqttOptions::new(client_id, &config.host, config.port);
    opts.set_keep_alive(KEEP_ALIVE);
    opts.set_clean_session(true);

    if let (Some(user), Some(pass)) = (&config.user, &config.pass) {
        opts.set_credentials(user, pass);
    }

    if config.tls {
        tracing::warn!(
            "MQTT_TLS=true is configured but TLS is not yet implemented; connecting without TLS"
        );
    }

    let availability_topic = topics::availability_topic(&config.bms_name);
    opts.set_last_will(LastWill::new(
        &availability_topic,
        b"offline".to_vec(),
        QoS::AtLeastOnce,
        true,
    ));

    opts
}

#[cfg(test)]
pub(super) mod internals {
    use std::time::Duration;
    pub const INITIAL_BACKOFF: Duration = super::INITIAL_BACKOFF;
    pub const MAX_BACKOFF: Duration = super::MAX_BACKOFF;
    pub const STABLE_THRESHOLD: Duration = super::STABLE_THRESHOLD;
}
