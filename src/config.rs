use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;

use crate::jkbms::JkBmsConfig;
use crate::mqtt::MqttConfig;

/// Global configuration passed to the supervisor / healthcheck at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub healthcheck: bool,
    pub healthcheck_server: bool,
    pub healthcheck_socket: Option<String>,

    /// Tracing filter string (e.g. "info", "jkbms2mqtt=debug").
    pub log_level: String,

    pub jkbms: JkBmsConfig,
    pub mqtt: MqttConfig,
}

// Every field carries `#[arg(long, env = "…")]`, so a single field definition
// drives both CLI parsing and env fallback. Precedence: CLI flag > env var > default.
#[derive(Parser, Debug)]
#[command(name = "jkbms2mqtt", version, about = "JK BMS → MQTT bridge")]
struct Cli {
    /// Run the self-healthcheck (MQTT ping/pong) and exit 0 on success, 1 on failure.
    #[arg(long)]
    pub healthcheck: bool,

    /// Start the healthcheck IPC server (needed for the `--healthcheck` client to
    /// query health status). Off by default; the Dockerfile enables it.
    #[arg(long)]
    pub healthcheck_server: bool,

    /// Override the healthcheck UDS name (default: `jkbms2mqtt-<bms-name>.sock`).
    /// Applies to both the server (`--healthcheck-server`) and the client
    /// (`--healthcheck`); both sides must agree.
    #[arg(long)]
    pub healthcheck_socket: Option<String>,

    /// Logical BMS name — used as MQTT topic prefix and HA entity ID prefix.
    #[arg(long, env = "BMS_NAME")]
    pub bms_name: String,

    /// Serial device path (e.g. /dev/ttyUSB0, COM3, /dev/tty.usbserial-XXXX).
    #[arg(long, env = "BMS_DEVICE")]
    pub bms_device: String,

    /// Modbus slave address of the BMS.
    #[arg(long, env = "BMS_SLAVE_ID", default_value_t = 1)]
    pub bms_slave_id: u8,

    /// MQTT broker hostname or IP.
    #[arg(long, env = "MQTT_HOST")]
    pub mqtt_host: String,

    /// MQTT broker port.
    #[arg(long, env = "MQTT_PORT", default_value_t = 1883)]
    pub mqtt_port: u16,

    /// MQTT username.
    #[arg(long, env = "MQTT_USER")]
    pub mqtt_user: Option<String>,

    /// MQTT password. Prefer MQTT_PASS_FILE (or the env var) over a CLI flag to
    /// avoid leaking the secret in `ps` and shell history.
    #[arg(long, env = "MQTT_PASS", hide_env_values = true)]
    pub mqtt_pass: Option<String>,

    /// Path to a file whose contents are used as the MQTT password. Mutually
    /// exclusive with `--mqtt-pass` / `MQTT_PASS`. Trailing newline is stripped.
    /// Ideal for Docker `secrets:` mounts and Kubernetes secret volumes.
    #[arg(long, env = "MQTT_PASS_FILE")]
    pub mqtt_pass_file: Option<PathBuf>,

    /// Enable TLS for the MQTT connection.
    #[arg(long, env = "MQTT_TLS", default_value_t = false)]
    pub mqtt_tls: bool,

    /// Override the MQTT client ID (default: `jkbms2mqtt-<bms-name>`).
    #[arg(long, env = "MQTT_CLIENT_ID")]
    pub mqtt_client_id: Option<String>,

    /// HA MQTT discovery prefix.
    #[arg(long, env = "HA_DISCOVERY_PREFIX", default_value = "homeassistant")]
    pub ha_discovery_prefix: String,

    /// Tracing filter (e.g. "info", "jkbms2mqtt=debug").
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,
}

impl Config {
    pub fn parse() -> Result<Self> {
        Self::from_cli(Cli::parse())
    }

    fn from_cli(cli: Cli) -> Result<Self> {
        let mqtt_pass = resolve_mqtt_pass(cli.mqtt_pass, cli.mqtt_pass_file)?;

        Ok(Config {
            healthcheck: cli.healthcheck,
            healthcheck_server: cli.healthcheck_server,
            healthcheck_socket: cli.healthcheck_socket,
            log_level: cli.log_level,
            jkbms: JkBmsConfig {
                bms_device: cli.bms_device,
                slave_id: cli.bms_slave_id,
            },
            mqtt: MqttConfig {
                bms_name: cli.bms_name,
                host: cli.mqtt_host,
                port: cli.mqtt_port,
                user: cli.mqtt_user,
                pass: mqtt_pass,
                tls: cli.mqtt_tls,
                client_id: cli.mqtt_client_id,
                discovery_prefix: cli.ha_discovery_prefix,
            },
        })
    }
}

fn resolve_mqtt_pass(inline: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    match (inline, file) {
        (Some(_), Some(_)) => {
            bail!("MQTT_PASS and MQTT_PASS_FILE are mutually exclusive; set only one")
        }
        (Some(p), None) => Ok(Some(p)),
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read MQTT_PASS_FILE at {}", path.display()))?;
            // Passwords may legitimately contain leading/embedded whitespace,
            // so trim only the trailing newline written by `echo` / editors.
            let trimmed = raw.trim_end_matches(['\n', '\r']).to_string();
            Ok(Some(trimmed))
        }
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_cli() -> Cli {
        Cli {
            healthcheck: false,
            healthcheck_server: false,
            healthcheck_socket: None,
            bms_name: "test".into(),
            bms_device: "/dev/null".into(),
            bms_slave_id: 1,
            mqtt_host: "localhost".into(),
            mqtt_port: 1883,
            mqtt_user: None,
            mqtt_pass: None,
            mqtt_pass_file: None,
            mqtt_tls: false,
            mqtt_client_id: None,
            ha_discovery_prefix: "homeassistant".into(),
            log_level: "info".into(),
        }
    }

    #[test]
    fn both_pass_sources_error() {
        let mut cli = base_cli();
        cli.mqtt_pass = Some("inline".into());
        cli.mqtt_pass_file = Some(PathBuf::from("/nonexistent"));
        let err = Config::from_cli(cli).unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn file_pass_trims_trailing_newline() {
        let path = std::env::temp_dir().join(format!("jkbms2mqtt-pass-{}.txt", std::process::id()));
        std::fs::write(&path, "secret\n").unwrap();

        let mut cli = base_cli();
        cli.mqtt_pass_file = Some(path.clone());
        let config = Config::from_cli(cli).unwrap();

        std::fs::remove_file(&path).ok();
        assert_eq!(config.mqtt.pass.as_deref(), Some("secret"));
    }

    #[test]
    fn no_pass_yields_none() {
        let config = Config::from_cli(base_cli()).unwrap();
        assert!(config.mqtt.pass.is_none());
    }
}
