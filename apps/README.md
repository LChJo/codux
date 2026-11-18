# Codux Apps

This directory contains runnable Codux applications. Shared protocol, runtime, terminal, and transport code lives in `../crates`; app directories should only own product-specific UI, packaging, platform integration, or process entry points.

## Applications

| Path | Role | Runtime |
| --- | --- | --- |
| `desktop/` | Rust + GPUI desktop app. Owns the primary UI, local workspace orchestration, AI CLI sessions, local terminal adapter, and host-side remote runtime. | Rust |
| `mobile/` | Flutter mobile controller. Connects to a Codux host, renders remote runtime state, and sends user intent. | Flutter + Rust FFI |
| `agent/` | Headless controlled-agent entry point. Uses shared protocol, transport, runtime, and PTY crates without GPUI. | Rust |

## Commands

Run from the repository root:

```bash
just desktop
just mobile android
just mobile ios
just agent -- --version
just test
```

Use app-local commands only when working inside that app's native toolchain, such as `flutter test` in `apps/mobile`.

## Release Flow

This fork publishes its desktop and headless Agent builds through [its GitHub release workflow](../docs/release.md).

| Product | Source | Release repository | Trigger | Output |
| --- | --- | --- | --- | --- |
| Desktop and Agent | `apps/desktop`, `apps/agent`, and shared `crates` | `LChJo/codux` | Push a version tag in this repository | Windows and macOS installers, Agent binaries, and desktop update manifests |
| Mobile | `apps/mobile` and shared `crates` | `duxweb/codux-flutter` | Push the same `vX.Y.Z` tag in the mobile release shell repository | Android APK release and iOS TestFlight upload; workflow checks out `duxweb/codux@vX.Y.Z` |
| Service | `duxweb/codux-service` | `duxweb/codux-service` | Push `vX.Y.Z` tag in the service release repository | Linux/macOS service tarballs and GHCR Docker image |

The mobile and service rows describe the upstream projects' own pipelines. A tag in this fork triggers the desktop and Agent release described in the [release guide](../docs/release.md).

## Ownership Rules

- UI code belongs in an app directory.
- Shared protocol names, payload shapes, transport rules, terminal state, and reusable runtime models belong in `../crates`.
- Do not duplicate Iroh transport selection, terminal sequence handling, or remote PTY restore logic in app code.
- Keep generated build output out of version control.
