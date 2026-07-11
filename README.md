# jkbms2mqtt

[![CI](https://github.com/SergYmb/jkbms2mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/SergYmb/jkbms2mqtt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Container: GHCR](https://img.shields.io/badge/container-ghcr.io-blue)](https://github.com/SergYmb/jkbms2mqtt/pkgs/container/jkbms2mqtt)

JK-BMS wired RS485 to MQTT bridge for Home Assistant. Runs as a standalone Docker container or native binary.

A small Rust service, shipped as a ready-to-use, prebuilt multi-arch Docker image that runs alongside a Home Assistant Docker installation. It talks to a JK BMS over a wired serial RS485 link, typically a USB-to-RS485 adapter, decodes its frames, and publishes the readings as Home Assistant MQTT-Discovery entities. Control switches are exposed back as MQTT command topics so HA can operate the BMS.

## What it does

- Polls the BMS every 5 seconds for live data: pack voltage / current / power, SoC, SoH, per-cell voltages and resistances, temperatures, balancer state, alarms.
- Auto-detects cell count from the BMS configuration frame and creates the matching set of HA entities.
- Registers everything via HA MQTT Discovery — no manual YAML.
- Exposes BMS control switches Home Assistant can turn on and off — manually or from automations.

## MQTT / Home Assistant data

Everything is published under `<BMS_NAME>/…` topics and registered via HA MQTT Discovery — entities appear in Home Assistant with no manual YAML:

- **Sensors** — pack voltage / current / power / SoC / SoH / capacity, per-cell voltage and resistance, cell aggregates (min / max / average / delta), temperatures, balancer current, alarm list.
- **Binary sensor** — `balancing_active`.
- **Switches** — `charging` and `balancing` (turned on and off from HA).
- **Diagnostics** — MQTT / BMS reconnect counters, last-update age.
- **Availability** — `<BMS_NAME>/availability` LWT (`online` / `offline`).
- **JSON snapshot** — `<BMS_NAME>/state` — flat JSON of every sensor value, one publish per poll cycle.

See [`doc/mqtt-topics.md`](doc/mqtt-topics.md) for the full topic reference — names, units, value formats, discovery payloads, and command topic contracts.

## Target hardware

- **BMS:** JK-PB2A16S20P (and likely other PB-series 16S-capable units running similar firmware).
- **Interface:** any USB-to-RS485 adapter exposed as `/dev/ttyUSB*`.
- **Deployment platform:**
  - **Docker:** a [ready-to-use, prebuilt multi-arch image](#install-docker-compose) for `linux/arm64/v8` (Raspberry Pi class, SBCs) and `linux/amd64` (x86_64 servers, NAS boxes).
  - **Native binary:** Linux, Windows and macOS, for development and single-host installs — see [Native builds (Linux / Windows / macOS)](#native-builds-linux--windows--macos).

## Reliability

The service is built to keep running through short serial disconnections, USB re-enumerations, and EMI-induced noise on the RS485 line. Corrupt frames are dropped, the BMS and MQTT broker are both reconnected automatically with backoff, and Home Assistant availability is held `online` across brief hiccups so entities don't flap.  
Switch commands from Home Assistant are queued and retried across these brief outages — a toggle issued manually or by an automation is held for up to 10 seconds and delivered once the link recovers, instead of being silently dropped.

---

## Install (Docker Compose)

Docker Compose is the recommended installation method. A prebuilt multi-arch image is published to GitHub Container Registry for every release, and Docker pulls the build matching your host architecture (`linux/amd64` or `linux/arm64/v8`) automatically.

You can set up the jkbms2mqtt container in its own standalone Docker Compose stack, or add it to the same stack as your Home Assistant Docker container and an MQTT broker such as Mosquitto.

See the Docker Compose example below, and refer to the [Configuration](#configuration) section for more details.

### Docker Compose example

```yaml
services:
  jkbms2mqtt:
    image: ghcr.io/sergymb/jkbms2mqtt:latest
    container_name: jkbms2mqtt

    environment:
      # Required
      - BMS_NAME=my_jk_bms
      - BMS_DEVICE=/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0
      - MQTT_HOST=192.168.1.10

      # Optional — uncomment as needed
      # - MQTT_PORT=1883
      # - MQTT_USER=
      # - MQTT_PASS=
      # - BMS_SLAVE_ID=1
      # - LOG_LEVEL=info

    # RS485 USB adapter exposed to the container.
    # Use the same path on both sides so the by-id symlink resolves inside.
    devices:
      - "/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0:/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0"

    # Grant access to serial ports.
    # Verify the group name with: ls -l /dev/ttyUSB0
    group_add:
      - dialout
```

> **Device path note.** `devices:` populates the container's `/dev` at start time. A same-path re-enumeration (`ttyUSB0` → `ttyUSB0`) survives without any change. A path shift (`ttyUSB0` → `ttyUSB1`) does not — use the by-id symlink form above, or bind-mount the host `/dev` and add the cgroup rule `c 188:* rmw`.

---

## Configuration

Every option can be set as a command-line flag or as an environment variable — CLI flags take precedence, env vars are the fallback, then the built-in defaults. Docker Compose deployments typically stay on env vars; native runs usually pass CLI flags. Run `jkbms2mqtt --help` for the same list with rendered defaults.

| Env var | CLI flag | Required | Default | Description |
|---|---|---|---|---|
| `BMS_NAME` | `--bms-name` | **Yes** | — | Logical BMS name — used as MQTT topic prefix and HA entity ID prefix |
| `BMS_DEVICE` | `--bms-device` | **Yes** | — | Serial device path (e.g. `/dev/ttyUSB0`, `COM3`, `/dev/tty.usbserial-XXXX`) |
| `MQTT_HOST` | `--mqtt-host` | **Yes** | — | MQTT broker hostname or IP address |
| `MQTT_PORT` | `--mqtt-port` | No | `1883` | MQTT broker port |
| `MQTT_USER` | `--mqtt-user` | No | — | MQTT username |
| `MQTT_PASS` | `--mqtt-pass` | No | — | MQTT password. Prefer `MQTT_PASS_FILE` for Docker / production; the env-var form is fine for env-file mounts. Never pass on the CLI — it leaks in `ps` |
| `MQTT_PASS_FILE` | `--mqtt-pass-file` | No | — | Path to a file whose contents are the MQTT password. Mutually exclusive with `MQTT_PASS`. Trailing newline is stripped. Pairs with Docker `secrets:` and Kubernetes secret volumes |
| `MQTT_TLS` | `--mqtt-tls` | No | `false` | Enable TLS for the MQTT connection |
| `MQTT_CLIENT_ID` | `--mqtt-client-id` | No | `jkbms2mqtt-<BMS_NAME>` | Override the MQTT client ID |
| `BMS_SLAVE_ID` | `--bms-slave-id` | No | `1` | Modbus slave address of the BMS |
| `BMS_BAUD` | — | No | `115200` | Serial baud rate (not currently configurable at runtime — hardcoded in the transport layer) |
| `HA_DISCOVERY_PREFIX` | `--ha-discovery-prefix` | No | `homeassistant` | HA MQTT discovery prefix |
| `LOG_LEVEL` | `--log-level` | No | `info` | Tracing filter (e.g. `info`, `jkbms2mqtt=debug`) |

Three healthcheck-related flags are CLI-only (no env-var counterpart):

- `--healthcheck` — runs the ping/pong self-test subcommand against a running instance and exits 0 on healthy, 1 otherwise. Used by the Docker `HEALTHCHECK` line.
- `--healthcheck-server` — enables the IPC server that answers `--healthcheck` queries. Off by default; the shipped `Dockerfile` enables it in `ENTRYPOINT`. Native runs that want their own healthcheck loop must pass this explicitly.
- `--healthcheck-socket <NAME>` — overrides the abstract-namespace UDS name used by the server and the client (default: `jkbms2mqtt-<BMS_NAME>.sock`). Both sides must be given the same value.

---

## Native builds (Linux / Windows / macOS)

The service is a plain Rust binary that builds and runs natively on Linux, Windows, macOS Intel, and macOS Apple Silicon from the same source tree. Docker is not required for these hosts.

**Toolchain (all supported hosts):** install Rust via <https://rustup.rs>. Minimum supported version: 1.85 (see `Cargo.toml`). Distro-packaged `rustc` is fine if it meets that version; otherwise use rustup.

**Build:**

```sh
cargo build --release
```

Output binary: `target/release/jkbms2mqtt` on Linux and macOS, `target\release\jkbms2mqtt.exe` on Windows.

### Linux

Build prerequisites (needed by cargo to link the final binary and build C-based dependencies):

- **Debian / Ubuntu:** `sudo apt install build-essential pkg-config`
- **Fedora / RHEL:** `sudo dnf install gcc pkgconf-pkg-config`
- **Arch:** `sudo pacman -S --needed base-devel`
- **Alpine:** `sudo apk add build-base musl-dev`

USB-serial adapters usually enumerate as `/dev/ttyUSB0` (CH340 / CP210x / FTDI adapters — kernel drivers are built-in on any recent distro). Prefer the `/dev/serial/by-id/…` symlink for a stable path that survives re-enumeration.

The device is typically owned by `root:dialout` (Debian family) or `root:uucp` (Arch / Fedora). Add your user to that group once, then log out and back in:

```sh
sudo usermod -aG dialout "$USER"    # or: uucp, depending on distro
```

Run with CLI flags:

```sh
./target/release/jkbms2mqtt \
  --bms-name my_jk_bms \
  --bms-device /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0 \
  --mqtt-host 192.168.1.10
```

### Windows

Install a USB-serial VCP driver matched to your RS485 adapter chipset — the adapter then appears as `COMx` in Device Manager:

- **CP210x** (most common) — Silicon Labs VCP driver
- **FTDI FT232** — FTDI VCP driver
- **CH340 / CH341** — WCH driver

Run from PowerShell with CLI flags:

```powershell
.\target\release\jkbms2mqtt.exe `
  --bms-name my_jk_bms `
  --bms-device COM3 `
  --mqtt-host 192.168.1.10
```

### macOS (Intel and Apple Silicon)

`cargo build --release` produces a binary for the machine's native arch — Intel Macs get `x86_64-apple-darwin`, Apple Silicon Macs get `aarch64-apple-darwin`. No `--target` flag is needed for a same-machine build.

Recent macOS releases bundle drivers for FTDI and CP210x adapters; CH340-based adapters typically need the WCH DriverKit installer. The adapter appears as `/dev/tty.usbserial-XXXX` — list attached ports with:

```sh
ls /dev/tty.usbserial-*
```

Run with CLI flags:

```sh
./target/release/jkbms2mqtt \
  --bms-name my_jk_bms \
  --bms-device /dev/tty.usbserial-A1B2C3D4 \
  --mqtt-host 192.168.1.10
```

> **Serial permissions on macOS.** The user account that owns the login session generally has access to `/dev/tty.usbserial-*`. If you see a "Permission denied" error, check that no other program (Arduino IDE, an open `screen` session, etc.) is holding the port.

---

## Build and run with Docker

> **Note.** A prebuilt multi-arch image is published to GitHub Container Registry for every release, see the [Install (Docker Compose)](#install-docker-compose) section. The steps below build the image from source instead.

The `Dockerfile` cross-compiles inside the builder stage (pinned to `$BUILDPLATFORM`), so both `linux/arm64/v8` and `linux/amd64` images build at native host speed regardless of which platform buildx is asked to produce. The same file supports single-arch and multi-arch builds without modification.

### One-time setup: multi-platform builder

`docker buildx` needs a builder that supports multi-platform. Create one once and select it as the default:

```sh
docker buildx create --use --name multiarch-builder
```

### Local build (single arch, loadable into `docker`)

`--load` imports a built image into your local Docker daemon, but it only supports one architecture at a time. Pick the arch you want to run locally:

```sh
# arm64 image — Raspberry Pi, SBCs, Apple Silicon under Docker Desktop
docker buildx build --platform linux/arm64/v8 -t jkbms2mqtt:latest --load .

# amd64 image — x86_64 servers, NAS, Intel/AMD desktops
docker buildx build --platform linux/amd64 -t jkbms2mqtt:latest --load .
```

### Multi-arch build and publish to a registry

Produce a single manifest containing both arm64 and amd64 images. Consumers pulling the tag get the arch that matches their host automatically.

Log in to the registry:

```sh
docker login {registry}
```

Build and push both platforms in one shot:

```sh
docker buildx build \
  --platform linux/amd64,linux/arm64/v8 \
  -t {registry}/{owner}/jkbms2mqtt:latest \
  --push \
  .
```

---

## Logging

`LOG_LEVEL` is a [`tracing-subscriber` env filter](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) — the same syntax as `RUST_LOG`. Levels progress `error` < `warn` < `info` < `debug` < `trace`; `trace` is reserved for two high-volume channels:

- **BMS wire bytes** (`jkbms2mqtt::jkbms=trace`) — every serial TX/RX with direction, length, elapsed-ms, and full hex. Frame 0x03 password fields are always redacted.
- **Full MQTT payloads** (`jkbms2mqtt::mqtt=trace`) — every published and received topic with its payload as UTF-8.

Everything else lives at `info` (state transitions), `warn` (recoverable errors), or `debug` (poll cycles, snapshot decisions, MQTT publish counts).

Set `LOG_LEVEL` in the `environment:` block of the compose file:

```yaml
# Default — connection state and errors only
- LOG_LEVEL=info

# App-wide debug (poll cycles, publish counts, availability transitions)
- LOG_LEVEL=jkbms2mqtt=debug

# Debug the BMS side: see every serial byte on the wire
- LOG_LEVEL=jkbms2mqtt=debug,jkbms2mqtt::jkbms=trace

# Debug the MQTT side: see every published / received payload
- LOG_LEVEL=jkbms2mqtt=debug,jkbms2mqtt::mqtt=trace

# Full trace of both channels (very verbose)
- LOG_LEVEL=jkbms2mqtt=debug,jkbms2mqtt::jkbms=trace,jkbms2mqtt::mqtt=trace
```

Changes take effect on container restart (`docker compose restart jkbms2mqtt`).

---

## Documentation

- [`REQUIREMENTS.md`](REQUIREMENTS.md) — functional and non-functional requirements, configuration, HA entity catalog.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — Rust / Tokio design: actor-based concurrency, module layout, testability seams, USB resilience implementation, Docker build, test strategy.
- [`doc/jkbms-protocol.md`](doc/jkbms-protocol.md) — wire-level protocol reference: Modbus framing, JK frame layout, register map, working byte sequences for tests.
- [`doc/mqtt-topics.md`](doc/mqtt-topics.md) — MQTT topic contracts published and subscribed by the service.
