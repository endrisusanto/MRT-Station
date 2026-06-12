# Production Adapter Checklist

The repository intentionally ships with a simulated provider because approved SDK headers, protobuf files, backend credentials, certificates, and hardware specifications are not present in the workspace.

Before production release:

1. Replace the development authenticator in `AgentState` with an HTTPS backend adapter using the approved trust chain and timeout policy.
2. Replace the simulated `DeviceProvider` with USB, CDC ACM, and Wi-Fi providers selected by the production VID/PID and firmware matrix.
3. Replace the generic protobuf envelope payload definitions with the approved schemas, then add byte-for-byte golden fixtures.
4. Store refresh/session secrets through Windows Credential Manager and Linux Secret Service. Never expose them through Tauri commands.
5. Add Linux peer credential verification and a Windows named-pipe security descriptor restricted to the interactive user and service identities.
6. Add signed update manifests, atomic replacement, rollback, and package signing.
7. Execute contract tests against the legacy system for every model, firmware, transport, and operation.

The simulator must be disabled in release builds once production adapters are integrated.

Release builds default to `EM_AGENT_MODE=production` and currently fail closed because those adapters have not been supplied. `EM_AGENT_MODE=simulator` must never be configured by production packages.

## Backend contract baseline

The production authenticator boundary is implemented with native-root TLS and no certificate bypass:

- `POST {EM_BACKEND_URL}/v1/sessions` with `username` and `password`.
- Response fields: `userId`, `displayName`, `expiresAt`, opaque `sessionToken`, and `permissions`.
- `DELETE {EM_BACKEND_URL}/v1/sessions/current` with the opaque bearer token.
- `EM_BACKEND_TIMEOUT_SECONDS` defaults to 15 and is constrained to 1-120 seconds.
- Plain HTTP is rejected unless `EM_BACKEND_ALLOW_HTTP=true`, which is only for isolated contract tests.

This wire contract is a placeholder boundary and must be reconciled with the approved backend specification before production enablement.
