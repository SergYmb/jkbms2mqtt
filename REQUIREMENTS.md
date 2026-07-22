# jkbms2mqtt — Requirements

JK-PB BMS USB-RS485 → MQTT for Home Assistant.

## Target Hardware
- **BMS model:** JK-PB2A16S20P (active balancer 2A, 16S capable, 8 cells connected)
- **Interface:** USB-RS485 adapter (FTDI Dual RS232-HS) → `/dev/ttyUSB*`

## Companion documents
- **Protocol reference:** [`doc/jkbms-protocol.md`](doc/jkbms-protocol.md)
- **MQTT topic contracts:** [`doc/mqtt-topics.md`](doc/mqtt-topics.md)
- **Implementation design:** [`ARCHITECTURE.md`](ARCHITECTURE.md)

---

## Functional Requirements

### FR-1: BMS Communication
- Open serial device at configurable path (default `/dev/ttyUSB0`), 115200 baud, 8N1
- Send Modbus RTU FC `0x10` trigger writes to request each JK frame type on startup and on each poll cycle (see protocol doc: frame trigger registers)
- Receive and parse JK auto-push frames identified by `55 AA EB 90` magic header, dispatch by frame type byte
- Send Modbus RTU FC `0x03` requests to poll the alarm register every configurable interval
- Send Modbus RTU FC `0x10` frames on write commands received from MQTT
- Share one serial port across all operations (trigger writes, frame reads, alarm polls, control writes)
- All bus operations (trigger writes, frame reads, alarm polls, control writes) must be serialized in FIFO order; concurrent writes are not permitted (RS485 is half-duplex). A minimum gap (`INTERFRAME_GAP`, compile-time const in `connection_manager.rs`) must be observed between consecutive Modbus frames to satisfy RS485 inter-frame silence.
- Validate every received JK frame against its fixed 300-byte length, magic header, and trailing 8-bit modulo checksum at byte 299 (see protocol doc, JK Frame Format). Discard and log at `warn` on mismatch
- After every successful control-register write, immediately re-trigger Frame `0x01` (Configuration) and publish the switch state read back from offsets `0x76` (charging) / `0x7E` (balancing). This is the read-after-write confirmation pattern; optimistic UI state must not be published without confirmation. Because the Configuration trigger is part of the Write handler itself, the confirmed state always arrives in the same operation
- Reconnect serial port on error with exponential backoff — see NFR-5 for USB-level resilience requirements (EMI-induced disconnects, path re-enumeration, stuck-state escalation)

### FR-2: Startup Sequence
1. Open serial port
2. Trigger and receive Frame `0x03` (device info) → extract model, serial, hardware/software version
3. Trigger and receive Frame `0x01` (config) → extract `cell_count`, initial switch states, `battery_capacity`
4. Register all HA entities via MQTT Discovery using `cell_count` from step 3
5. Begin poll loops (run concurrently, each serialized through the shared serial bus per FR-1):
   - Every 5 s (`OPERATIONAL_POLL_INTERVAL`, const): trigger Frame `0x02` and poll the alarm register
   - Every 20 s (`CONFIG_POLL_INTERVAL`, const): trigger Frame `0x01` to pick up config changes made via the JK phone app
   - Every 30 s (`DEVICE_INFO_POLL_INTERVAL`, const): trigger Frame `0x03` to detect BMS swap / firmware change

### FR-3: HA Entities

Full entity catalog (topics, units, value formats) lives in [`doc/mqtt-topics.md`](doc/mqtt-topics.md); source frame offsets are in [`doc/jkbms-protocol.md`](doc/jkbms-protocol.md). Requirements-level rules not covered there:

- **Cell entities & aggregates.** `cell_N_voltage` / `cell_N_resistance` published for `N ∈ [1, cell_count]` only (cell_count from Frame 0x01 at startup). Aggregates (`average`, `min`, `max`, `delta`, `min_cell`, `max_cell`) are computed exclusively over active cells; inactive slots read 0 and are excluded.
- **Computed sensors.** `total_power = total_voltage × total_current` (signed). `total_runtime` is `total_runtime_seconds` rendered as ISO-8601 duration. `last_update_age` is seconds since the last successful Frame 0x02 parse.
- **Temperature numbering.** T3 is intentionally skipped so entity numbering matches the JK BMS Mobile App's T1/T2/T4/T5 labels (offset `0xFE` mirrors MOS temperature and is not exposed).
- **Switches.** `charging` writes `0x1070`, reads Frame 0x01 offset `0x76`. `balancing` writes `0x1078`, reads Frame 0x01 offset `0x7E`. `0x1078` was empirically verified on JK-PB2A16S20P / firmware 15.41 — older references label it "Charging float mode" but toggling it changes actual balancer running state (visible via `balancing_active`, Frame 0x02 offset `0xAC`).
- **Diagnostic entities** carry `entity_category: diagnostic`.

#### HA entity metadata

All discovery payloads carry `device_class` and `state_class` per the mapping below.

| Entity ID(s) | device_class | state_class |
|---|---|---|
| `total_voltage`, `cell_*_voltage`, `cell_voltage_{average,min,max,delta}` | `voltage` | `measurement` |
| `total_current`, `balancing_current` | `current` | `measurement` |
| `total_power` | `power` | `measurement` |
| `state_of_charge`, `state_of_health` | `battery` | `measurement` |
| `mos_temperature`, `temperature_sensor_*` | `temperature` | `measurement` |
| `capacity_remaining`, `cell_*_resistance` | none | `measurement` |
| `total_cycle_capacity`, `charging_cycles`, `power_cycle_count`, `jkbms_reconnect_count`, `mqtt_reconnect_count` | none | `total_increasing` |
| `total_runtime_seconds` | `duration` | `total_increasing` |
| `last_update_age` | `duration` | `measurement` |
| `cell_voltage_min_cell`, `cell_voltage_max_cell`, `alarm_list`, `total_runtime` | none | none |

> Frame 0x03 offset `0x26` (apparent "uptime") was excluded from v1: live capture showed a non-uniform update rate (0 increments over 30 s, then ~7.3/s), inconsistent with a steady seconds counter. Tracked under Open Items.

### FR-4: MQTT / HomeAssistant Integration
- HA MQTT Discovery: `<HA_DISCOVERY_PREFIX>/<component>/<BMS_NAME>/<object_id>/config`
- Re-publish discovery payloads on reconnect
- Publish retained per-entity state after each successful Frame `0x02` / alarm poll cycle
- Publish a JSON snapshot to `<BMS_NAME>/state` after every poll cycle, retained, containing all numeric/string sensor values keyed by `entity_id`. Per-entity state topics continue to be published — the snapshot is additive
- Subscribe to command topics for entities listed under FR-3 Switches and FR-3 Number; write to BMS on receipt. Do NOT subscribe to command topics for entities in the "Planned (gated)" subsection
- MQTT QoS: discovery payloads QoS 1 + retained; per-entity and snapshot state QoS 0 + retained; command subscriptions QoS 1
- Availability topic (LWT = offline on disconnect; online on connect). All discovery payloads include `availability_topic` pointing at `<BMS_NAME>/availability` with `payload_available: "online"` / `payload_not_available: "offline"`
- Device info in all discovery payloads: manufacturer `JIKONG`, model / hardware version / software version / serial number from Frame `0x03`
- Entity IDs follow pattern `<BMS_NAME>_<entity_id>` (e.g. `my_jk_bms_total_voltage`)

### FR-5: Configuration

All config via environment variables (12-factor); optional TOML file as fallback.

| Variable | Description | Default |
|---|---|---|
| `BMS_DEVICE` | Serial device path | `/dev/ttyUSB0` |
| `BMS_SLAVE_ID` | Modbus slave address | `1` |
| `MQTT_HOST` | Broker hostname | — |
| `MQTT_PORT` | Broker port | `1883` |
| `MQTT_USER` | Broker username | — |
| `MQTT_PASS` | Broker password | — |
| `MQTT_TLS` | Enable TLS | `false` |
| `MQTT_CLIENT_ID` | MQTT client identifier | `jkbms2mqtt` |
| `HA_DISCOVERY_PREFIX` | HA MQTT discovery prefix | `homeassistant` |
| `BMS_NAME` | Logical name of this BMS — used as MQTT topic prefix and HA entity ID prefix | **required** (e.g. `my_jk_bms`) |
| `LOG_LEVEL` | trace / debug / info / warn / error | `info` |

---

## Non-Functional Requirements

### NFR-1: Implementation language
jkbms2mqtt is implemented in Rust on Tokio.

### NFR-2: Docker deployment
- **Target platform:** `linux/arm64/v8`
- Device access via a `/dev` bind-mount — the container always sees the host's current device tree, so a `by-id` symlink stays valid across USB re-enumeration (see NFR-5)
- Run as non-root; user in `dialout` group
- Health check: the binary's availability state is `online`.

### NFR-3: Reliability
- Validate every JK frame: exact 300-byte length, `55 AA EB 90` magic header, and trailing 8-bit modulo checksum at byte 299. On any mismatch, log at `warn` and await next frame — never crash on a malformed or incomplete frame
- If no Frame `0x02` received within 3× poll interval, publish availability = offline
- MQTT reconnect with exponential backoff; re-publish discovery and availability on reconnect
- Serial reconnect: see NFR-5

### NFR-4: Observability
- Structured logs: `info` for startup/shutdown and reconnection attempts; `warn`/`error` for disconnections and errors; `debug` for BMS/MQTT operational event summaries; `trace` for raw serial bytes and MQTT payloads.
- MQTT availability (FR-4), diagnostic HA sensors (FR-3), and the Docker healthcheck (NFR-2) together surface liveness at the app, HA, and container level.

### NFR-5: USB / Serial Resilience

EMI-induced USB disconnects and re-enumeration (e.g. `ttyUSB0` → `ttyUSB1`) is normal operation.

The Docker `/dev` bind-mount is chosen over a single-device `devices:` mapping because the latter resolves the `by-id` symlink only once, at container start, so a re-enumeration onto a different node leaves the mapping stale until the container is restarted. The bind-mount instead gives the container a live view of the host's device tree, so the app can reconnect without issues.

- **Disconnect** on: any `io::Error` on `read`/`write`, `read` returning 0 bytes, or `RECONNECT_THRESHOLD` (=5) consecutive soft failures (parse error / read timeout). Successful Frame `0x02` parse resets the counter.
- **On disconnect:** close fd, discard partial-frame buffer, fail in-flight writes with transient error, publish `availability = offline` immediately.
- **Reopen** with backoff `RECONNECT_BACKOFF` = 250/1000/1750/2500/4500 ms (10 s total), then `RECONNECT_BACKOFF_MAX` = 5000 ms cap. Retry forever; reset on success.
- **Resync** after reopen: replay FR-2 startup (Frame `0x03` → `0x01` → `0x02`).
- No USB-level recovery or path discovery — those are kernel/Compose concerns.

---

## Out of Scope (v1)
- Writing arbitrary BMS configuration registers (thresholds, temperatures, etc.)
- Multiple BMS units on one adapter
- CAN bus interface
- Web UI / REST API
- Discharge switch control (may damage inverter)
