// ── Alarm bit descriptions ─────────────────────────────────────────────────

pub const ALARM_DESCRIPTIONS: &[&str] = &[
    "Balancing resistance too high",          // bit 0
    "MOS over-temperature protection",        // bit 1
    "Cell count mismatch",                    // bit 2
    "Abnormal current sensor",                // bit 3
    "Cell over-voltage protection",           // bit 4
    "Battery over-voltage protection",        // bit 5
    "Overcurrent charge protection",          // bit 6
    "Charge short-circuit protection",        // bit 7
    "Over-temperature charge protection",     // bit 8
    "Low temperature charge protection",      // bit 9
    "Internal communication anomaly",         // bit 10
    "Cell under-voltage protection",          // bit 11
    "Battery under-voltage protection",       // bit 12
    "Overcurrent discharge protection",       // bit 13
    "Discharge short-circuit protection",     // bit 14
    "Over-temperature discharge protection",  // bit 15
    "Charge MOS anomaly",                     // bit 16
    "Discharge MOS anomaly",                  // bit 17
    "GPS disconnected",                       // bit 18
    "Authorization password change required", // bit 19
    "Discharge activation failure",           // bit 20
    "Battery over-temperature alarm",         // bit 21
    "Temperature sensor anomaly",             // bit 22
    "Parallel module anomaly",                // bit 23
];

// ── Device Info (Frame 0x03) ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct JkBmsDeviceInfo {
    pub model: String,
    pub hardware_version: String,
    pub software_version: String,
    pub serial_number: String,
    pub power_cycle_count: u32,
}

// ── Configuration Options (Frame 0x01) ────────────────────────────────────

#[derive(Debug)]
pub struct JkBmsConfigOptions {
    pub cell_count: u32,
    pub charging_switch: bool,
    pub balance_switch: bool,
    pub battery_capacity_ah: f64,
    pub smart_sleep_voltage_v: f64,
    pub cell_undervoltage_protection_v: f64,
    pub cell_overvoltage_protection_v: f64,
    pub balance_trigger_voltage_v: f64,
}

// ── Operational Data (Frame 0x02) ─────────────────────────────────────────

pub const TOTAL_CELL_SLOTS: usize = 16;

#[derive(Debug)]
pub struct JkBmsOperationalData {
    pub cell_voltages_v: [f64; TOTAL_CELL_SLOTS],
    pub cell_resistances_ohm: [f64; TOTAL_CELL_SLOTS],
    pub total_voltage_v: f64,
    pub total_current_a: f64,
    pub soc_pct: u8,
    pub soh_pct: u8,
    pub capacity_remaining_ah: f64,
    pub total_cycle_capacity_ah: f64,
    pub charging_cycles: i32,
    pub total_runtime_s: u32,
    pub mos_temperature_c: f64,
    pub temperature_sensor_1_c: f64,
    pub temperature_sensor_2_c: f64,
    pub temperature_sensor_4_c: f64,
    pub temperature_sensor_5_c: f64,
    pub balancing_current_a: f64,
    pub balancing_active: bool,
    pub charging_switch: bool,
}

// ── BMS data type selector ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JkBmsDataType {
    ConfigOptions,
    OperationalData,
    DeviceInfo,
    Alarms,
}

// ── BMS data payload ──────────────────────────────────────────────────────

/// Inner type — symmetric with JkBmsDataType. Carried by JkBmsEvents::Data.
#[derive(Debug)]
pub enum JkBmsData {
    ConfigOptions(JkBmsConfigOptions),
    OperationalData(Box<JkBmsOperationalData>),
    DeviceInfo(JkBmsDeviceInfo),
    Alarms(u32),
}
