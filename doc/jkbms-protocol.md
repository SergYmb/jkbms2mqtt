# JK-PB BMS RS485 Protocol Reference

**Target hardware:** JK-PB2A16S20P  
**Interface:** USB-RS485 (FTDI Dual RS232-HS)  
**Physical layer:** 115200 baud, 8N1, half-duplex RS485

---

## Protocol Overview

The JK-PB BMS uses two protocols on the same RS485 bus:

| Operation | Direction | Protocol | Description |
|---|---|---|---|
| Request frame | host → BMS | Modbus RTU FC `0x10` | Write to a trigger register |
| Frame response | BMS → host | JK proprietary | 300-byte frame starting with `55 AA EB 90`, sent **before** the Modbus ACK |
| Trigger ACK | BMS → host | Modbus RTU FC `0x10` | Standard 8-byte write ACK, sent **after** the JK frame |
| Read alarms | host → BMS → host | Modbus RTU FC `0x03` | Standard register read, returns 32-bit alarm flags |
| Write control | host → BMS | Modbus RTU FC `0x10` | Write a uint32 to a control register; BMS responds with a standard Modbus FC `0x10` ACK |

---

## Modbus Register Map

### Frame Trigger Registers (FC `0x10`)

Send a write of 1 register, value `0x0000`, to trigger the BMS to emit the corresponding JK frame.

| Frame | Register (hex) | Register (dec) | Response frame type |
|---|---|---|---|
| Device info | `0x161C` | 5660 | `0x03` |
| Configuration | `0x161E` | 5662 | `0x01` |
| Operational data | `0x1620` | 5664 | `0x02` |

**Request wire format** (11 bytes):
```
[SlaveID] [0x10] [0x16] [reg_low] [0x00] [0x01] [0x02] [0x00] [0x00] [CRC_L] [CRC_H]
```

> **ACK order.** Unlike control writes where only an ACK is returned, trigger writes receive two responses: the 300-byte JK proprietary frame first (starting with `55 AA EB 90`), then the standard 8-byte Modbus FC `0x10` ACK. Implementations must read the JK frame first, then drain the ACK — failing to drain it will misalign subsequent reads.

### Alarm Register (FC `0x03`)

| Register (hex) | Register (dec) | Count | Encoding |
|---|---|---|---|
| `0x12A0` | 4768 | 2 registers | uint32 **Big-Endian** |

**Request wire format** (8 bytes):
```
[SlaveID] [0x03] [0x12] [0xA0] [0x00] [0x02] [CRC_L] [CRC_H]
```

**Response wire format** (9 bytes):
```
[SlaveID] [0x03] [0x04] [D3] [D2] [D1] [D0] [CRC_L] [CRC_H]
                        └──── uint32 BE ────┘
```

### Write Control Registers (FC `0x10`)

All write values are uint32 (2 registers, 4 bytes, Big-Endian data). Value `0` = off/false, `1` = on/true.

| Register (hex) | Control | Values |
|---|---|---|
| `0x1070` | Charge switch | `0` = disable, `1` = enable |
| `0x1078` | Balance switch | `0` = disable, `1` = enable |
| TBD | Total battery capacity | integer × 1000 (stored in mAh) |

> Register `0x1078` was verified empirically on JK-PB2A16S20P firmware 15.41 — writing toggles the balancer's actual running state (visible at Frame 0x02 offset `0xAC` `balancing_active`). Older JK reference docs label this register "Charging float mode"; that label is wrong, or applies to a different firmware family. See the "Working Examples" and "Empirical Verification" sections below.

**Request wire format** (13 bytes):
```
[SlaveID] [0x10] [reg_H] [reg_L] [0x00] [0x02] [0x04] [val_B3] [val_B2] [val_B1] [val_B0] [CRC_L] [CRC_H]
                                                       └──────── uint32 BE ────────┘
```

The 4-byte data section MUST encode the full uint32 value in big-endian order. The earlier convention of "value only in the low byte" only works for the on/off switches (values 0–1) and silently fails for `battery_capacity` (typically tens of thousands of mAh).

**Worked examples**

Charge enable (`0x1070`, value `1`, slave ID `1`):
```
01 10 10 70 00 02 04 00 00 00 01 [CRC_L] [CRC_H]
```

Battery capacity write (placeholder register `0xTTTT`, value 280 Ah = 280000 mAh = `0x00044570`, slave ID `1`):
```
01 10 TT TT 00 02 04 00 04 45 70 [CRC_L] [CRC_H]
```

### CRC

Standard Modbus CRC16, polynomial `0xA001`, appended **little-endian** to every request and response.

---

## JK Frame Format

All three BMS response frames share the same 6-byte header:

```
[0x55] [0xAA] [0xEB] [0x90] [type] [0x00] [data ...]
 ───────── magic (4) ──────   (1)    (1)   offset 6
```

| `type` | Frame | Size |
|---|---|---|
| `0x03` | Device info (static) | 300 bytes |
| `0x01` | Configuration (setup) | 300 bytes |
| `0x02` | Operational data (live) | 300 bytes |

All multi-byte values within the data section are **little-endian** unless noted otherwise.

### Frame Trailer and Validation

Every frame is exactly **300 bytes** total. The last byte (offset 299) is an 8-bit checksum:

```
checksum = sum(bytes[0..=298]) mod 256
```

The parser MUST validate, in order, on every received frame:
1. Length is exactly 300 bytes
2. Bytes 0..=3 equal the magic `55 AA EB 90`
3. The 8-bit modulo sum of bytes 0..=298 equals byte 299

On any failure, discard the frame and log at `warn`. Never act on partially validated data.

> **Verified against live hardware** (JK-PB2A16S20P, firmware 15.41, 13 Jun 2026). The 300-byte length and checksum at byte 299 were confirmed from a full frame capture. Earlier versions of this doc stated 308 bytes — that figure came from a timeout-split that collected the 300-byte JK frame together with the trailing 8-byte Modbus ACK into a single buffer.

---

## Frame 0x03 — Device Info

Triggered once at startup. Contains static identification data.

| Offset | Hex | Size | Type | Field | Notes |
|---|---|---|---|---|---|
| 6 | 0x06 | 13 | ASCII | model | e.g. `JK_PB2A16S20P` |
| 22 | 0x16 | 3 | ASCII | hardware_version | e.g. `15A` |
| 30 | 0x1E | 5 | ASCII | software_version | e.g. `15.41` |
| 38 | 0x26 | 4 | uint32le | total_runtime_mirror | same 1 Hz seconds-since-first-power-on counter as Frame 0x02 offset `0xC2`. Confirmed from live capture: Frame 0x03 read 45,895,200 s; Frame 0x02 read 38 s later = 45,895,238 s. "Non-uniform" appearance in earlier captures was a polling-cadence artefact (Frame 0x03 polled every ~30 s). Redundant with Frame 0x02 — not exposed as a v1 entity. |
| 42 | 0x2A | 4 | uint32le | power_cycle_count | number of power-on events |
| 46 | 0x2E | 13 | ASCII | serial_number | ASCII digits + NUL padding; e.g. `99999999999` (real serial redacted) |
| 62 | 0x3E | 8 | ASCII | password_1 | (not exposed) |
| 78 | 0x4E | 8 | ASCII | manufacturing_date | (not exposed) |
| 102 | 0x66 | 8 | ASCII | brand | e.g. `JIKONG` |
| 118 | 0x76 | 8 | ASCII | password_2 | (not exposed) |
| 184 | 0xB8 | 1 | uint8 | uart1_protocol | (not exposed) |
| 185 | 0xB9 | 1 | uint8 | can_protocol | (not exposed) |
| 234 | 0xEA | 1 | uint8 | lcd_buzzer_trigger | (not exposed) |
| 238 | 0xEE | 4 | uint32le | lcd_buzzer_trigger_value | (not exposed) |
| 242 | 0xF2 | 4 | uint32le | lcd_buzzer_release_value | (not exposed) |
| 266 | 0x10A | 1 | uint8 | request_charge_voltage_time | ×0.1 s (not exposed) |
| 267 | 0x10B | 1 | uint8 | request_float_voltage_time | ×0.1 s (not exposed) |

---

## Frame 0x01 — Configuration

Triggered once at startup. Contains protection thresholds, settings, cell count, and switch states.

| Offset | Hex | Size | Type | Field | Scale | Notes |
|---|---|---|---|---|---|---|
| 6 | 0x06 | 4 | int32le | smart_sleep_voltage | /1000 V | |
| 10 | 0x0A | 4 | int32le | cell_undervoltage_protection | /1000 V | |
| 14 | 0x0E | 4 | int32le | cell_undervoltage_recovery | /1000 V | |
| 18 | 0x12 | 4 | int32le | cell_overvoltage_protection | /1000 V | |
| 22 | 0x16 | 4 | int32le | cell_overvoltage_recovery | /1000 V | |
| 26 | 0x1A | 4 | int32le | balance_trigger_voltage | /1000 V | cell-to-cell voltage delta above which balancing activates |
| 30 | 0x1E | 4 | int32le | cell_soc100_voltage | /1000 V | voltage at 100% SOC |
| 34 | 0x22 | 4 | int32le | cell_soc0_voltage | /1000 V | voltage at 0% SOC |
| 38 | 0x26 | 4 | int32le | cell_request_charge_voltage | /1000 V | |
| 42 | 0x2A | 4 | int32le | cell_request_float_voltage | /1000 V | |
| 46 | 0x2E | 4 | int32le | power_off_voltage | /1000 V | |
| 50 | 0x32 | 4 | int32le | max_charge_current | /1000 A | |
| 54 | 0x36 | 4 | int32le | charge_overcurrent_delay | ×1 s | |
| 58 | 0x3A | 4 | int32le | charge_overcurrent_recovery | ×1 s | |
| 62 | 0x3E | 4 | int32le | max_discharge_current | /1000 A | |
| 66 | 0x42 | 4 | int32le | discharge_overcurrent_delay | ×1 s | |
| 70 | 0x46 | 4 | int32le | discharge_overcurrent_recovery | ×1 s | |
| 74 | 0x4A | 4 | int32le | short_circuit_recovery | ×1 s | |
| 78 | 0x4E | 4 | int32le | max_balance_current | /1000 A | |
| 82 | 0x52 | 4 | int32le | charge_overtemp_protection | /10 °C | |
| 86 | 0x56 | 4 | int32le | charge_overtemp_recovery | /10 °C | |
| 90 | 0x5A | 4 | int32le | discharge_overtemp_protection | /10 °C | |
| 94 | 0x5E | 4 | int32le | discharge_overtemp_recovery | /10 °C | |
| 98 | 0x62 | 4 | int32le | charge_undertemp_protection | /10 °C | |
| 102 | 0x66 | 4 | int32le | charge_undertemp_recovery | /10 °C | |
| 106 | 0x6A | 4 | int32le | mos_overtemp_protection | /10 °C | |
| 110 | 0x6E | 4 | int32le | mos_overtemp_recovery | /10 °C | |
| **114** | **0x72** | **4** | **int32le** | **cell_count** | ×1 | **active cell count — drives entity creation** |
| 118 | 0x76 | 4 | int32le | charging_switch | 0/1 | authoritative switch enable state |
| 122 | 0x7A | 4 | int32le | discharging_switch | 0/1 | (not exposed to HA) |
| 126 | 0x7E | 4 | int32le | balance_switch | 0/1 | authoritative switch enable state |
| **130** | **0x82** | **4** | **int32le** | **total_battery_capacity** | **/1000 Ah** | **`battery_capacity` number entity** |
| 134 | 0x86 | 4 | int32le | short_circuit_delay | ×1 s | |
| 138 | 0x8A | 4 | int32le | balance_starting_voltage | /1000 V | minimum per-cell voltage below which balancing will not run (distinct from `balance_trigger_voltage`) |
| 158 | 0x9E | 4 | int32le | connection_wire_resistance | /1000 Ω | |
| 270 | 0x10E | 4 | int32le | device_address | ×1 | slave ID confirmation |
| 282 bit 4 | 0x11A | — | bool | display_always_on | — | (not exposed) |
| 282 bit 7 | 0x11A | — | bool | smart_sleep | — | (not exposed) |
| 282 bit 8 | 0x11A | — | bool | disable_pcl_module | — | (not exposed) |
| 282 bit 9 | 0x11A | — | bool | balance_switch_enabled | — | write via reg `0x1078`; authoritative source is Frame 0x01 offset 0x7E |
| 283 bit 1 | 0x11B | — | bool | timed_stored_data | — | (not exposed) |

---

## Frame 0x02 — Operational Data

Triggered every poll cycle. Primary source for all HA sensor updates.

### Cell voltages and resistances

Cell slots beyond `cell_count` (from Frame 0x01 offset 114) read `0` and **must be excluded** from all calculations and entity creation.

| Offset formula | Hex formula | Size | Type | Field | Scale |
|---|---|---|---|---|---|
| 6 + (N−1)×2, N=1..16 | 0x06 + (N−1)×2 | 2 | uint16le | cell N voltage | /1000 V |
| 80 + (N−1)×2, N=1..16 | 0x50 + (N−1)×2 | 2 | int16le | cell N resistance | /1000 Ω |

> Cell resistance is signed `int16le` in the wire format but observed values are non-negative. jkbms2mqtt clamps any negative reading to `0`.

### Pack data

| Offset | Hex | Size | Type | Field | Scale | HA entity |
|---|---|---|---|---|---|---|
| 234 | 0xEA | 2 | uint16le | total_voltage | /100 V | `total_voltage` |
| 158 | 0x9E | 4 | int32le | total_current | /1000 A (signed) | `total_current` |
| — | — | — | computed | power | total_voltage × total_current W | `total_power` |
| 173 | 0xAD | 1 | uint8 | soc | ×1 % | `state_of_charge` |
| 190 | 0xBE | 1 | uint8 | soh | ×1 % | `state_of_health` |
| 174 | 0xAE | 4 | int32le | capacity_remaining | /1000 Ah | `capacity_remaining` |
| 178 | 0xB2 | 4 | int32le | capacity_actual | /1000 Ah | (informational, not exposed) |
| 182 | 0xB6 | 4 | int32le | charge_cycle_count | ×1 | `charging_cycles` |
| 186 | 0xBA | 4 | int32le | total_cycle_capacity | /1000 Ah | `total_cycle_capacity` |
| 194 | 0xC2 | 4 | uint32le | total_runtime | ×1 s → format | `total_runtime` |

**Power note:** an unsigned `uint32le` power field exists earlier in the frame but is discarded — it cannot represent negative values during discharge. Always compute `total_power = total_voltage × total_current` instead.

**Runtime format:** raw seconds formatted as ISO-8601 duration, e.g. `"P530DT4H12M"`.

### Temperature

| Offset | Hex | Size | Type | Field | Scale | HA entity |
|---|---|---|---|---|---|---|
| 144 | 0x90 | 2 | int16le | mos_temperature | /10 °C | `mos_temperature` |
| 162 | 0xA2 | 2 | int16le | temperature_sensor_1 | /10 °C | `temperature_sensor_1` |
| 164 | 0xA4 | 2 | int16le | temperature_sensor_2 | /10 °C | `temperature_sensor_2` |
| 254 | 0xFE | 2 | int16le | mos_temperature_mirror | /10 °C | **not exposed** — always equals offset 0x90; corresponds to app's "T3" slot with no physical sensor connected |
| 256 | 0x100 | 2 | int16le | temperature_sensor_4 | /10 °C | `temperature_sensor_4` (app: "Battery T4") |
| 258 | 0x102 | 2 | int16le | temperature_sensor_5 | /10 °C | `temperature_sensor_5` (app: "Battery T5") |

> **Note:** The JK BMS Mobile App labels these sensors MOS Temp, Battery T1, Battery T2, Battery T4, Battery T5 — T3 is absent because no physical sensor is connected at that slot on this pack. Offset `0xFE` (254) returns the same value as the MOS temperature at offset `0x90`; it is parsed for diagnostic purposes only and not emitted as an HA entity. Entity numbering skips `temperature_sensor_3` to match the app. An earlier version of this doc listed 254/258 based on a third-party reference table; the correct offsets 256/258 were confirmed from live capture 2026-06-13.

### Balancer and switch states

| Offset | Hex | Size | Type | Field | Scale | HA entity |
|---|---|---|---|---|---|---|
| 170 | 0xAA | 2 | int16le | balancing_current | /1000 A | `balancing_current` |
| 172 | 0xAC | 1 | uint8 | balancing_active | 0/1 | source for `balancing_active` binary_sensor — whether the balancer is currently running, independent of the switch enable state at Frame 0x01 offsets 0x76/0x7E |
| 198 | 0xC6 | 1 | uint8 | charging_switch | 0/1 | MOS output state — not authoritative for the switch setting; switch state sourced from Frame 0x01 offset 0x76 |
| 199 | 0xC7 | 1 | uint8 | discharging_switch | 0/1 | (not exposed) |
| 200 | 0xC8 | 1 | uint8 | balance_switch (not used) | 0/1 | MOS output state — not parsed; switch state sourced from Frame 0x01 offset 0x7E |

Switch enable states are the **authoritative source from Frame 0x01** (offsets 0x76 / 0x7E). Frame 0x02 offsets 0xC6 / 0xC8 track MOS output state, not the user-configured switch setting, and are not used as the switch-state source.

---

## Alarm Bit Definitions

Alarm value: uint32 Big-Endian from register `0x12A0`. Bit = 1 means fault active.

| Bit | Code name | Description |
|---|---|---|
| 0 | `AlarmWireRes` | Balancing resistance too high |
| 1 | `AlarmMosOTP` | MOS over-temperature protection |
| 2 | `AlarmCellQuantity` | Cell count mismatch |
| 3 | `AlarmCurSensorErr` | Abnormal current sensor |
| 4 | `AlarmCellOVP` | Cell over-voltage protection |
| 5 | `AlarmBatOVP` | Battery over-voltage protection |
| 6 | `AlarmChOCP` | Overcurrent charge protection |
| 7 | `AlarmChSCP` | Charge short-circuit protection |
| 8 | `AlarmChOTP` | Over-temperature charge protection |
| 9 | `AlarmChUTP` | Low temperature charge protection |
| 10 | `AlarmCPUAuxCommuErr` | Internal communication anomaly |
| 11 | `AlarmCellUVP` | Cell under-voltage protection |
| 12 | `AlarmBatUVP` | Battery under-voltage protection |
| 13 | `AlarmDchOCP` | Overcurrent discharge protection |
| 14 | `AlarmDchSCP` | Discharge short-circuit protection |
| 15 | `AlarmDchOTP` | Over-temperature discharge protection |
| 16 | `AlarmChargeMOS` | Charge MOS anomaly |
| 17 | `AlarmDischargeMOS` | Discharge MOS anomaly |
| 18 | `GPSDisconnected` | GPS disconnected |
| 19 | `ModifyPWD` | Authorization password change required |
| 20 | `DischargeOnFailed` | Discharge activation failure |
| 21 | `BatteryOverTempAlarm` | Battery over-temperature alarm |
| 22 | `AlarmTempSensorErr` | Temperature sensor anomaly |
| 23 | `AlarmParallelModuleErr` | Parallel module anomaly |

`alarm_list` HA entity value: comma-separated descriptions of active bits, or empty string when alarm value is `0`. When bits 22 or 23 are active, render them using the descriptions above.

---

## Working Examples (for unit tests)

These byte sequences were captured from a live session against a real JK-PB2A16S20P running firmware 15.41 and are suitable as fixtures for jkbms2mqtt's frame-builder and parser tests.

**PII redaction rule** (applies to every byte sequence in this section and to any future capture committed to this repo):
- Frame 0x03 password fields at offsets `0x3E` and `0x76` (8 bytes each) → replace with `58 58 58 58 58 58 58 58` (ASCII `"XXXXXXXX"`).
- Frame 0x03 serial-number field at offset `0x2E` (13 bytes) → replace with `39 39 39 39 39 39 39 39 39 39 39 00 00` (ASCII `"99999999999"` + NUL padding).
- Frame 0x03 offset `0x26` 4-byte field (whose semantics are not yet verified) → replace with `00 00 00 00` in examples.
- Never quote the BMS serial number string anywhere in this repo.

CRCs on request frames (which contain no PII) are reproduced verbatim from the capture and are the exact bytes the BMS accepts.

### Trigger Frame 0x03 (device info)

Request (11 bytes):
```
01 10 16 1C 00 01 02 00 00 D3 CD
```

Response, first 50 bytes shown (full frame is 308 bytes; remainder including the password fields at offsets `0x3E` / `0x76` is elided and must be `58×8` per the redaction rule in any committed fixture):
```
55 AA EB 90 03 00 4A 4B 5F 50 42 32 41 31 36 53
32 30 50 00 00 00 31 35 41 00 00 00 00 00 31 35
2E 34 31 00 00 00 00 00 00 00 27 00 00 00 39 39
39 39
```

Annotated:
- `55 AA EB 90` — magic
- `03 00` — type byte + reserved
- `4A 4B 5F 50 42 32 41 31 36 53 32 30 50` at offset 6 — ASCII `"JK_PB2A16S20P"` (model)
- `31 35 41` at offset 22 — ASCII `"15A"` (hardware version)
- `31 35 2E 34 31` at offset 30 — ASCII `"15.41"` (software version)
- `00 00 00 00` at offset 38 — redacted "uptime" field (real value not preserved; field excluded from v1, see Open Items)
- `27 00 00 00` at offset 42 — `power_cycle_count` = 39
- `39 39 39 39 ...` at offset 46 — redacted serial number stub

### Trigger Frame 0x01 (configuration)

Request (11 bytes):
```
01 10 16 1E 00 01 02 00 00 D2 2F
```

Response, first 50 bytes:
```
55 AA EB 90 01 00 48 0D 00 00 46 0A 00 00 6E 0A
00 00 42 0E 00 00 AA 0D 00 00 05 00 00 00 AB 0D
00 00 5A 0A 00 00 AC 0D 00 00 16 0D 00 00 F6 09
00 00
```

Annotated (Frame 0x01 has no PII; bytes are reproduced verbatim):
- `55 AA EB 90 01 00` — header
- offset 6: `smart_sleep_voltage` = 0x00000D48 / 1000 = 3.400 V
- offset 10: `cell_undervoltage_protection` = 0x00000A46 / 1000 = 2.630 V
- offset 14: `cell_undervoltage_recovery` = 0x00000A6E / 1000 = 2.670 V
- offset 18: `cell_overvoltage_protection` = 0x00000E42 / 1000 = 3.650 V
- offset 22: `cell_overvoltage_recovery` = 0x00000DAA / 1000 = 3.498 V
- offset 26: `balance_trigger_voltage` = 0x00000005 / 1000 = 0.005 V (5 mV delta)
- offset 30: `cell_soc100_voltage` = 0x00000DAB / 1000 = 3.499 V
- offset 34: `cell_soc0_voltage` = 0x00000A5A / 1000 = 2.650 V
- offset 38: `cell_request_charge_voltage` = 0x00000DAC / 1000 = 3.500 V
- offset 42: `cell_request_float_voltage` = 0x00000D16 / 1000 = 3.350 V
- offset 46: `power_off_voltage` = 0x000009F6 / 1000 = 2.550 V

### Trigger Frame 0x02 (operational)

Request (11 bytes):
```
01 10 16 20 00 01 02 00 00 D6 F1
```

Response, first 24 bytes (cells 1–8 of an 8S pack, with cells 9–16 reading 0 as expected):
```
55 AA EB 90 02 00 8F 0D 93 0D 90 0D 90 0D 8F 0D
90 0D 8F 0D 8F 0D 00 00
```

Annotated:
- `55 AA EB 90 02 00` — header
- cell 1 at offset 6: 0x0D8F / 1000 = 3.471 V
- cell 2 at offset 8: 0x0D93 / 1000 = 3.475 V
- cells 3–8: 3.472, 3.472, 3.471, 3.472, 3.471, 3.471 V
- offset 22: cell 9 = `00 00` → inactive slot (confirms `cell_count` = 8 from Frame 0x01)

### Read alarm register

Request (8 bytes):
```
01 03 12 A0 00 02 C1 51
```

Response, no alarms active (9 bytes):
```
01 03 04 00 00 00 00 FA 33
```

Note: endianness is big-endian per Modbus FC `0x03` convention (register data is always transmitted high-byte-first). An all-zero payload is consistent with this; it does not require a non-zero capture to confirm what the Modbus spec mandates.

### Write charging switch (register `0x1070`)

Disable (value `0x00000000`):
```
Request:  01 10 10 70 00 02 04 00 00 00 00 39 4B
Response: 01 10 10 70 00 02 44 D3
```

Enable (value `0x00000001`):
```
Request:  01 10 10 70 00 02 04 00 00 00 01 F8 8B
Response: 01 10 10 70 00 02 44 D3
```

### Write balance switch (register `0x1078`)

Disable (value `0x00000000`):
```
Request:  01 10 10 78 00 02 04 00 00 00 00 38 ED
Response: 01 10 10 78 00 02 C5 11
```

Enable (value `0x00000001`):
```
Request:  01 10 10 78 00 02 04 00 00 00 01 F9 2D
Response: 01 10 10 78 00 02 C5 11
```

> Earlier JK references label register `0x1078` as "Charging float mode". Live capture against firmware 15.41 confirms it toggles the balancer (visible via `balancing_active` at Frame 0x02 offset `0xAC`). jkbms2mqtt treats this register as the balance switch.

---

## Empirical Verification

The following items were confirmed against a live capture from a JK-PB2A16S20P running firmware 15.41.
