// ── Cell aggregates ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct CellAggregates {
    pub average_v: f64,
    pub min_v: f64,
    pub max_v: f64,
    pub delta_v: f64,
    pub min_cell: u32, // 1-based
    pub max_cell: u32, // 1-based
}

// ── Snapshot ───────────────────────────────────────────────────────────────

/// Flat snapshot of all sensor values after a Frame 0x02 + alarm cycle.
/// Used for per-entity MQTT publishes and the JSON snapshot topic.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    // Pack
    pub total_voltage_v: f64,
    pub total_current_a: f64,
    pub total_power_w: f64,
    pub soc_pct: u8,
    pub soh_pct: u8,
    pub capacity_remaining_ah: f64,
    pub total_cycle_capacity_ah: f64,
    pub battery_capacity_ah: f64,
    pub charging_cycles: i32,
    pub total_runtime_s: u32,
    pub total_runtime: String,

    // Cells — indexed 0 = cell 1
    pub cell_voltages_v: Vec<f64>,
    pub cell_resistances_ohm: Vec<f64>,
    pub cell_aggregates: Option<CellAggregates>,

    // Temperature
    pub mos_temperature_c: f64,
    pub temperature_sensor_1_c: f64,
    pub temperature_sensor_2_c: f64,
    pub temperature_sensor_4_c: f64,
    pub temperature_sensor_5_c: f64,

    // Balancer
    pub balancing_current_a: f64,
    pub balancing_active: bool,

    // Switches
    pub charging_switch: bool,
    pub balance_switch: bool,

    // Alarms
    pub alarm_raw: u32,
    pub alarm_list: String,

    // Diagnostics
    pub power_cycle_count: u32,
    pub last_update_age_s: u64,
    pub jkbms_reconnect_count: u32,
    pub mqtt_reconnect_count: u32,
}
