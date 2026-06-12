# EM Station

Rust and Tauri replacement baseline for the legacy EM Station and JP Agent Service.

## Included

- Rust workspace with shared domain models and typed errors.
- Versioned protobuf IPC framing with correlation IDs and a 1 MiB frame limit.
- Cross-platform agent endpoint: Unix socket on Linux and named pipe on Windows.
- Session lifecycle, permissions, asynchronous multi-device operations, cancellation, and per-device results.
- Replaceable `DeviceProvider` boundary with two simulated development devices.
- Production read-only USB and CDC/COM discovery filtered by an explicit VID/PID inventory.
- Replaceable backend authenticator with native-root HTTPS, bounded timeouts, typed status mapping, and opaque session tokens.
- Tauri v2 and React operator interface covering login, discovery, selection, token modes, expiry, install, remove, info, recovery, progress, and results.
- systemd, udev, tmpfiles, Windows service scripts, and CI baselines.

## Run locally

Terminal 1:

```bash
EM_AGENT_MODE=simulator cargo run -p em-agent
```

Terminal 2:

```bash
cd apps/station
npm install
npm run tauri dev
```

Simulator mode accepts any non-empty username and password. It is intended only for authorized development. Optimized agent builds default to production mode and refuse startup until approved backend and hardware adapters are configured. Credentials remain in agent memory only and are cleared on logout or process exit.

Production configuration is loaded from environment variables. The Linux service reads `/etc/em-station/agent.env`; see [`packaging/linux/agent.env.example`](packaging/linux/agent.env.example).

## Verify

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/station && npm ci && npm run build
```

## Release

The release script validates the workspace, bumps all application versions, commits, tags, and pushes. The pushed tag triggers GitHub Actions to publish Windows and Linux Tauri bundles plus standalone agent archives.

```bash
./release.sh patch
./release.sh minor
./release.sh 1.0.0
```

The configured release repository is `https://github.com/endrisusanto/MRT-Station.git`.

## Production status

This is a runnable development baseline, not a production-equivalent hardware implementation. The approved backend, device protocols, production protobuf schemas, certificates, and model matrix were not supplied in this workspace. Complete [the adapter checklist](docs/PRODUCTION_ADAPTERS.md) before hardware or production acceptance.
