use tokio::time::Instant;

use crate::domain::{CellAggregates, Snapshot};
use crate::jkbms::{
    ALARM_DESCRIPTIONS, JkBmsConfigOptions, JkBmsDeviceInfo, JkBmsOperationalData, TOTAL_CELL_SLOTS,
};

pub(crate) struct StateAggregator {
    device_info: Option<JkBmsDeviceInfo>,
    config_options: Option<JkBmsConfigOptions>,
    operational: Option<JkBmsOperationalData>,
    alarms_raw: Option<u32>,
    last_update_at: Option<Instant>,
}

impl StateAggregator {
    pub(crate) fn new() -> Self {
        Self {
            device_info: None,
            config_options: None,
            operational: None,
            alarms_raw: None,
            last_update_at: None,
        }
    }

    pub(crate) fn set_device_info(&mut self, info: JkBmsDeviceInfo) {
        self.last_update_at = Some(Instant::now());
        self.device_info = Some(info);
    }

    pub(crate) fn set_config_options(&mut self, opts: JkBmsConfigOptions) {
        self.last_update_at = Some(Instant::now());
        self.config_options = Some(opts);
    }

    pub(crate) fn set_operational(&mut self, op: JkBmsOperationalData, at: Instant) {
        self.last_update_at = Some(at);
        self.operational = Some(op);
    }

    pub(crate) fn set_alarms(&mut self, raw: u32) {
        self.last_update_at = Some(Instant::now());
        self.alarms_raw = Some(raw);
    }

    pub(crate) fn has_device_info(&self) -> bool {
        self.device_info.is_some()
    }

    pub(crate) fn has_config_options(&self) -> bool {
        self.config_options.is_some()
    }

    pub(crate) fn has_operational(&self) -> bool {
        self.operational.is_some()
    }

    pub(crate) fn device_info(&self) -> Option<&JkBmsDeviceInfo> {
        self.device_info.as_ref()
    }

    pub(crate) fn config_options(&self) -> Option<&JkBmsConfigOptions> {
        self.config_options.as_ref()
    }

    /// Build a Snapshot from the latest cached fragments. Returns `None` until both
    /// `ConfigOptions` (for `cell_count`) and at least one `OperationalData` have arrived.
    pub(crate) fn snapshot(&self) -> Option<Snapshot> {
        let config = self.config_options.as_ref()?;
        let op = self.operational.as_ref()?;
        let last_at = self.last_update_at?;

        let n = (config.cell_count as usize).min(TOTAL_CELL_SLOTS);
        let cell_voltages_v = op.cell_voltages_v[..n].to_vec();
        let cell_resistances_ohm = op.cell_resistances_ohm[..n].to_vec();

        let cell_aggregates = if !cell_voltages_v.is_empty() {
            Some(compute_cell_aggregates(&cell_voltages_v))
        } else {
            None
        };

        let alarm_raw = self.alarms_raw.unwrap_or(0);
        let alarm_list = decode_alarms(alarm_raw);
        let power_cycle_count = self.device_info.as_ref().map_or(0, |d| d.power_cycle_count);

        Some(Snapshot {
            total_voltage_v: op.total_voltage_v,
            total_current_a: op.total_current_a,
            total_power_w: total_power(op.total_voltage_v, op.total_current_a),
            soc_pct: op.soc_pct,
            soh_pct: op.soh_pct,
            capacity_remaining_ah: op.capacity_remaining_ah,
            total_cycle_capacity_ah: op.total_cycle_capacity_ah,
            battery_capacity_ah: config.battery_capacity_ah,
            charging_cycles: op.charging_cycles,
            total_runtime_s: op.total_runtime_s,
            total_runtime: format_iso8601(op.total_runtime_s),
            cell_voltages_v,
            cell_resistances_ohm,
            cell_aggregates,
            mos_temperature_c: op.mos_temperature_c,
            temperature_sensor_1_c: op.temperature_sensor_1_c,
            temperature_sensor_2_c: op.temperature_sensor_2_c,
            temperature_sensor_4_c: op.temperature_sensor_4_c,
            temperature_sensor_5_c: op.temperature_sensor_5_c,
            balancing_current_a: op.balancing_current_a,
            balancing_active: op.balancing_active,
            charging_switch: config.charging_switch,
            balance_switch: config.balance_switch,
            alarm_raw,
            alarm_list,
            power_cycle_count,
            last_update_age_s: last_update_age_secs(last_at),
            jkbms_reconnect_count: 0, // coordinator fills this in
            mqtt_reconnect_count: 0,  // coordinator fills this in
        })
    }
}

// ── Snapshot-building helpers ─────────────────────────────────────────────────

fn decode_alarms(value: u32) -> String {
    if value == 0 {
        return String::new();
    }
    let parts: Vec<&str> = ALARM_DESCRIPTIONS
        .iter()
        .enumerate()
        .filter(|(i, _)| value & (1 << i) != 0)
        .map(|(_, desc)| *desc)
        .collect();
    if parts.is_empty() {
        format!("unknown bits 0x{:08X}", value)
    } else {
        parts.join(", ")
    }
}

fn format_iso8601(secs: u32) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hours = rem / 3_600;
    let minutes = (rem % 3_600) / 60;
    format!("P{}DT{}H{}M", days, hours, minutes)
}

fn compute_cell_aggregates(voltages: &[f64]) -> CellAggregates {
    assert!(!voltages.is_empty(), "at least one active cell required");
    let mut min_v = f64::INFINITY;
    let mut max_v = f64::NEG_INFINITY;
    let mut min_cell = 1u32;
    let mut max_cell = 1u32;
    let mut sum = 0.0f64;

    for (i, &v) in voltages.iter().enumerate() {
        sum += v;
        if v < min_v {
            min_v = v;
            min_cell = (i + 1) as u32;
        }
        if v > max_v {
            max_v = v;
            max_cell = (i + 1) as u32;
        }
    }

    CellAggregates {
        average_v: sum / voltages.len() as f64,
        min_v,
        max_v,
        delta_v: max_v - min_v,
        min_cell,
        max_cell,
    }
}

fn total_power(voltage_v: f64, current_a: f64) -> f64 {
    voltage_v * current_a
}

fn last_update_age_secs(last_update_at: Instant) -> u64 {
    last_update_at.elapsed().as_secs()
}

#[cfg(test)]
pub(super) mod internals {
    use tokio::time::Instant;

    use crate::domain::CellAggregates;

    pub fn decode_alarms(value: u32) -> String {
        super::decode_alarms(value)
    }
    pub fn format_iso8601(secs: u32) -> String {
        super::format_iso8601(secs)
    }
    pub fn compute_cell_aggregates(voltages: &[f64]) -> CellAggregates {
        super::compute_cell_aggregates(voltages)
    }
    pub fn total_power(voltage_v: f64, current_a: f64) -> f64 {
        super::total_power(voltage_v, current_a)
    }
    pub fn last_update_age_secs(last_update_at: Instant) -> u64 {
        super::last_update_age_secs(last_update_at)
    }
}
