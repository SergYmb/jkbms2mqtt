# MQTT Topics Reference

This document describes all MQTT topics published and subscribed to by jkbms2mqtt.

---

## Topic Naming

```
<BMS_NAME>/<entity_id>/state      — published value (retained)
<BMS_NAME>/<entity_id>/set        — command input (switches and numbers)
<BMS_NAME>/state                  — JSON snapshot of all sensor values (retained)
<BMS_NAME>/availability           — online / offline (LWT)
```

`BMS_NAME` is a **required** environment variable (e.g. `my_jk_bms`).  
All state topics are published with the **retained** flag.

---

## HA MQTT Discovery

Discovery payloads are published once at startup and re-published on MQTT reconnect.

```
<HA_DISCOVERY_PREFIX>/<component>/<BMS_NAME>/<object_id>/config
```

| Component | Used for |
|---|---|
| `sensor` | All numeric and string read values |
| `binary_sensor` | `balancing_active` |
| `switch` | `charging`, `balancing` |

`HA_DISCOVERY_PREFIX` defaults to `homeassistant`.

Each discovery payload includes a `device` block with:
- `manufacturer`: `JIKONG`
- `model`: read from Frame 0x03 (e.g. `JK_PB2A16S20P`)
- `hw_version`: read from Frame 0x03 (e.g. `15A`)
- `sw_version`: read from Frame 0x03 (e.g. `15.41`)
- `serial_number`: read from Frame 0x03

All discovery payloads include `device_class` and `state_class` where applicable. See `REQUIREMENTS.md` FR-3 ("HA entity metadata") for the per-entity mapping. Diagnostic entities additionally carry `entity_category: diagnostic`.

**QoS and retention:**
- Discovery payloads: QoS 1, retained
- Per-entity state and JSON snapshot: QoS 0, retained
- Command subscriptions: QoS 1

---

## Availability

| Topic | Values | Notes |
|---|---|---|
| `<BMS_NAME>/availability` | `online` / `offline` | Set as MQTT LWT; published `online` after successful startup |

---

## Pack Sensors

| Topic | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/total_voltage/state` | V | float, 2 dp | `27.36` |
| `<BMS_NAME>/total_current/state` | A | float, 3 dp, signed | `-8.800` |
| `<BMS_NAME>/total_power/state` | W | float, 2 dp, signed | `-240.77` |
| `<BMS_NAME>/state_of_charge/state` | % | integer | `86` |
| `<BMS_NAME>/state_of_health/state` | % | integer | `100` |
| `<BMS_NAME>/capacity_remaining/state` | Ah | float, 1 dp | `240.8` |
| `<BMS_NAME>/total_cycle_capacity/state` | Ah | float, 1 dp | `25292.2` |
| `<BMS_NAME>/battery_capacity_ah/state` | Ah | float, 1 dp | `314.0` |
| `<BMS_NAME>/charging_cycles/state` | — | integer | `90` |
| `<BMS_NAME>/power_cycle_count/state` | — | integer | `47` |
| `<BMS_NAME>/total_runtime_seconds/state` | s | integer | `34906707` |
| `<BMS_NAME>/total_runtime/state` | — | ISO-8601 duration string | `P404DT0H18M` |

> **total_power** is always computed as `total_voltage × total_current`. Negative = discharging, positive = charging.

> **battery_capacity_ah** is the nominal capacity configured in the BMS (Frame 0x01). It is read-only; writing it is gated on Open Item 1 (write register unknown).

---

## Cell Sensors

`N` ranges from `1` to `cell_count` (auto-detected from Frame 0x01). Only active cells are published.

| Topic pattern | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/cell_N_voltage/state` | V | float, 3 dp | `3.467` |
| `<BMS_NAME>/cell_N_resistance/state` | Ω | float, 3 dp | `0.003` |

---

## Cell Aggregate Sensors

Computed over active cells only (cells beyond `cell_count` are excluded).

| Topic | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/cell_voltage_average/state` | V | float, 3 dp | `3.332` |
| `<BMS_NAME>/cell_voltage_min/state` | V | float, 3 dp | `3.328` |
| `<BMS_NAME>/cell_voltage_max/state` | V | float, 3 dp | `3.335` |
| `<BMS_NAME>/cell_voltage_delta/state` | V | float, 3 dp | `0.007` |
| `<BMS_NAME>/cell_voltage_min_cell/state` | — | integer (1-based) | `8` |
| `<BMS_NAME>/cell_voltage_max_cell/state` | — | integer (1-based) | `2` |

---

## Temperature Sensors

| Topic | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/mos_temperature/state` | °C | float, 1 dp | `19.3` |
| `<BMS_NAME>/temperature_sensor_1/state` | °C | float, 1 dp | `21.7` |
| `<BMS_NAME>/temperature_sensor_2/state` | °C | float, 1 dp | `21.4` |
| `<BMS_NAME>/temperature_sensor_4/state` | °C | float, 1 dp | `20.8` |
| `<BMS_NAME>/temperature_sensor_5/state` | °C | float, 1 dp | `20.5` |

---

## Balancer Sensors

| Topic | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/balancing_current/state` | A | float, 3 dp | `0.000` |

### Balancing Active (binary sensor)

| Topic | Values | Notes |
|---|---|---|
| `<BMS_NAME>/balancing_active/state` | `ON` / `OFF` | Balancer currently running; independent of the `balancing` switch |

---

## Alarm Sensor

| Topic | Value format | Example (no alarms) | Example (with alarms) |
|---|---|---|---|
| `<BMS_NAME>/alarm_list/state` | string | `` (empty) | `Cell over-voltage protection, Overcurrent charge protection` |

---

## Diagnostic Sensors

Published with `entity_category: diagnostic` in HA Discovery.

| Topic | Unit | Value format | Example |
|---|---|---|---|
| `<BMS_NAME>/last_update_age/state` | s | integer | `2` |
| `<BMS_NAME>/jkbms_reconnect_count/state` | — | integer | `3` |
| `<BMS_NAME>/mqtt_reconnect_count/state` | — | integer | `1` |

> An "uptime"-like field exists at Frame 0x03 offset `0x26` but was excluded from v1 because the live capture showed a non-uniform update rate inconsistent with a steady seconds counter. Tracked under Open Items in `REQUIREMENTS.md`.

---

## JSON Snapshot

| Topic | Format | Retention |
|---|---|---|
| `<BMS_NAME>/state` | JSON object | retained, QoS 0 |

Published once per poll cycle. Payload is a flat JSON object keyed by `entity_id`, containing every numeric and string sensor value (e.g. `voltage`, `current`, `cell_1_voltage`, `alarm_list`, `total_runtime`).

Per-entity topics remain primary; the snapshot is additive — useful for non-HA consumers that prefer a single subscription.

---

## Control Topics

### Switches

| State topic | Command topic | Payload values | Notes |
|---|---|---|---|
| `<BMS_NAME>/charging/state` | `<BMS_NAME>/charging/set` | `ON` / `OFF` | Charge MOSFET enable (write register `0x1070`) |
| `<BMS_NAME>/balancing/state` | `<BMS_NAME>/balancing/set` | `ON` / `OFF` | Balancer enable (write register `0x1078`, verified empirically — see protocol doc "Empirical Verification") |

State publishes `ON` or `OFF`. Command accepts `ON` / `OFF` (case-insensitive).  
HA MQTT Discovery configures `payload_on: "ON"` and `payload_off: "OFF"`.

### Number

> `battery_capacity` writable number entity is gated on Open Item 1 (battery capacity write register unknown) and is NOT published in v1. See "Topics Not Exposed in v1" below.

---

## Topics Not Exposed in v1

| Topic | Reason excluded |
|---|---|
| `<BMS_NAME>/battery_capacity/set` | Gated on Open Item 1 — write register unknown; read-only `battery_capacity_ah` sensor is exposed |
| Discharge MOSFET switch | Discharge control register not yet identified |
| Heater control | No heater on target hardware |
| Charge status / timing fields | Out of scope v1 |
| Protection threshold controls | Read-only for now; write registers unverified |
| Password fields | Not exposed for security |
| Static device info topics | Device info exposed via HA discovery payload device block, not individual topics |

---

## Full Topic Index (as example BMS_NAME='my_jk_bms')

```
my_jk_bms/availability
my_jk_bms/state
my_jk_bms/total_voltage/state
my_jk_bms/total_current/state
my_jk_bms/total_power/state
my_jk_bms/state_of_charge/state
my_jk_bms/state_of_health/state
my_jk_bms/capacity_remaining/state
my_jk_bms/total_cycle_capacity/state
my_jk_bms/battery_capacity_ah/state
my_jk_bms/charging_cycles/state
my_jk_bms/power_cycle_count/state
my_jk_bms/total_runtime_seconds/state
my_jk_bms/total_runtime/state
my_jk_bms/cell_1_voltage/state
my_jk_bms/cell_2_voltage/state
  ... (up to cell_count)
my_jk_bms/cell_1_resistance/state
my_jk_bms/cell_2_resistance/state
  ... (up to cell_count)
my_jk_bms/cell_voltage_average/state
my_jk_bms/cell_voltage_min/state
my_jk_bms/cell_voltage_max/state
my_jk_bms/cell_voltage_delta/state
my_jk_bms/cell_voltage_min_cell/state
my_jk_bms/cell_voltage_max_cell/state
my_jk_bms/mos_temperature/state
my_jk_bms/temperature_sensor_1/state
my_jk_bms/temperature_sensor_2/state
my_jk_bms/temperature_sensor_4/state
my_jk_bms/temperature_sensor_5/state
my_jk_bms/balancing_current/state
my_jk_bms/balancing_active/state
my_jk_bms/alarm_list/state
my_jk_bms/last_update_age/state
my_jk_bms/jkbms_reconnect_count/state
my_jk_bms/mqtt_reconnect_count/state
my_jk_bms/charging/state
my_jk_bms/charging/set
my_jk_bms/balancing/state
my_jk_bms/balancing/set
```
