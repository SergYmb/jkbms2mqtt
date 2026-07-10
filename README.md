# jkbms2mqtt

[![CI](https://github.com/SergYmb/jkbms2mqtt/actions/workflows/ci.yml/badge.svg)](https://github.com/SergYmb/jkbms2mqtt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Container: GHCR](https://img.shields.io/badge/container-ghcr.io-blue)](https://github.com/SergYmb/jkbms2mqtt/pkgs/container/jkbms2mqtt)

JK-BMS RS485 to MQTT bridge for Home Assistant. Runs as a standalone Docker container or native binary.

A small Rust service, packaged as a Docker container to run alongside a Home Assistant Docker installation. It talks to a JK BMS over USB-RS485, decodes its frames, and publishes the readings as Home Assistant MQTT-Discovery entities. Charge and balance switches are exposed back as MQTT command topics so HA can control the BMS.

## What it does

- Polls the BMS every 5 seconds for live data: pack voltage / current / power, SoC, SoH, per-cell voltages and resistances, temperatures, balancer state, alarms.
- Auto-detects cell count from the BMS configuration frame and creates the matching set of HA entities.
- Registers everything via HA MQTT Discovery — no manual YAML.

## MQTT / Home Assistant data

Everything is published under `<BMS_NAME>/…` topics and registered via HA MQTT Discovery — entities appear in Home Assistant with no manual YAML:

- **Sensors** — pack voltage / current / power / SoC / SoH / capacity, per-cell voltage and resistance, cell aggregates (min / max / average / delta), temperatures, balancer current, alarm list.
- **Binary sensor** — `balancing_active`.
- **Switches** — `charging` and `balancing` (writable via HA).
- **Diagnostics** — MQTT / BMS reconnect counters, last-update age.
- **Availability** — `<BMS_NAME>/availability` LWT (`online` / `offline`).
- **JSON snapshot** — `<BMS_NAME>/state` — flat JSON of every sensor value, one publish per poll cycle.

See [`doc/mqtt-topics.md`](doc/mqtt-topics.md) for the full topic reference — names, units, value formats, discovery payloads, and command topic contracts.

## Target hardware

- **BMS:** JK-PB2A16S20P (and likely other PB-series 16S-capable units running similar firmware).
- **Interface:** any USB-to-RS485 adapter exposed as `/dev/ttyUSB*`.
- **Deployment platform:** Docker container for `linux/arm64/v8` (Raspberry Pi class, SBCs) or `linux/amd64` (x86_64 servers, NAS boxes) — a single multi-arch image is built from the same `Dockerfile`. Native builds for Linux, Windows, macOS Intel, and macOS Apple Silicon are also supported for development and single-host installs — see [Native builds (Linux / Windows / macOS)](#native-builds-linux--windows--macos).

## Reliability

The service is built to keep running through short serial disconnections, USB re-enumerations, and EMI-induced noise on the RS485 line. Corrupt frames are dropped, the BMS and MQTT broker are both reconnected automatically with backoff, and Home Assistant availability is held `online` across brief hiccups so entities don't flap.

---

## Install (Docker)

Pull the prebuilt multi-arch image from GitHub Container Registry — Docker picks the arch that matches your host automatically:

```sh
docker pull ghcr.io/sergymb/jkbms2mqtt:latest
```

See the [Docker Compose example](#docker-compose-example) below for a full deployment, or [Build and run with Docker](#build-and-run-with-docker) to build locally.

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

## Handling the MQTT password safely

Ranked from safest to worst:

1. **`MQTT_PASS_FILE` pointing at a Docker / Kubernetes secret mount** — the credential is a `tmpfs` file that never appears in env, `docker inspect`, `ps`, or git.
2. **`MQTT_PASS` sourced from a mounted `env_file:` with `0600` perms** — the compose file references the env file by path; the file itself stays out of git.
3. **`MQTT_PASS` inline in `docker-compose.yml`** — works, but the compose file usually lives in git and the value is visible to anyone who can `docker inspect`.
4. **`--mqtt-pass <secret>` on the CLI** — visible to any local user via `ps aux` and lands in shell history. Avoid.

### Docker Compose with a real secret file

```yaml
secrets:
  mqtt_pass:
    file: ./secrets/mqtt_pass          # 0600, not committed to git

services:
  jkbms2mqtt:
    image: ghcr.io/sergymb/jkbms2mqtt:latest
    environment:
      - MQTT_PASS_FILE=/run/secrets/mqtt_pass
      - BMS_NAME=my_jk_bms
      - BMS_DEVICE=/dev/serial/by-id/usb-1a86_USB_Serial-if00-port0
      - MQTT_HOST=192.168.1.10
      - MQTT_USER=hass
    secrets:
      - mqtt_pass
    # …devices / group_add as in the full example below…
```

The container reads `/run/secrets/mqtt_pass` at startup and passes the value to the broker. Nothing sensitive is in the environment, in `docker inspect`, or in the compose file itself.

### Native runs

Store the password in a file the user account can read exclusively, then point at it:

```sh
umask 077
printf 'topsecret\n' > "$HOME/.config/jkbms2mqtt/mqtt_pass"

MQTT_PASS_FILE="$HOME/.config/jkbms2mqtt/mqtt_pass" \
  ./target/release/jkbms2mqtt \
    --bms-name my_jk_bms \
    --bms-device /dev/serial/by-id/usb-1a86_USB_Serial-if00-port0 \
    --mqtt-host 192.168.1.10 \
    --mqtt-user hass
```

On Windows the same idea applies — create a file readable only by the user account (via `icacls` or the Properties → Security dialog) and set `MQTT_PASS_FILE` to its path.

> **Username handling.** `MQTT_USER` deliberately does not have a `_FILE` variant — usernames aren't secrets in a typical setup. If you need it, ask; adding `MQTT_USER_FILE` is a small follow-up using the same pattern.

---

## Build and run with Docker

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

Or with environment variables:

```powershell
$env:BMS_NAME    = "my_jk_bms"
$env:BMS_DEVICE  = "COM3"
$env:MQTT_HOST   = "192.168.1.10"
.\target\release\jkbms2mqtt.exe
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

## Docker Compose example

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
