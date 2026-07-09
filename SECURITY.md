# Security policy

## Supported versions

Only the latest tagged release on the `main` branch receives security fixes.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security-sensitive reports.

Use GitHub's private vulnerability reporting: on this repo, go to the
**Security** tab → **Report a vulnerability**
(<https://github.com/SergYmb/jkbms2mqtt/security/advisories/new>). Include:

- A description of the issue and its impact.
- Steps to reproduce (a minimal config, log excerpts, or a proof-of-concept).
- The affected version or commit.

You should receive an acknowledgement within **7 days**. A fix or mitigation plan
will follow within **30 days** for confirmed issues; timelines for public
disclosure will be coordinated with the reporter.

## Scope

In scope:

- The `jkbms2mqtt` daemon itself (parsing untrusted BMS input, MQTT handling,
  configuration and secrets, the healthcheck IPC surface).
- The published `Dockerfile` and container images on GHCR.

Out of scope:

- Vulnerabilities in third-party dependencies unless directly exploitable
  through `jkbms2mqtt`'s own use of them (report those upstream first).
- Physical access to the RS485 bus or the host machine.
