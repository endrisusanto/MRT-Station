# Security Policy

- Never commit AD credentials, session secrets, certificates, raw tokens, device identifiers, or packet captures containing production data.
- Use sanitized fixtures for protocol and hardware tests.
- The UI must never persist passwords or session secrets.
- The agent rejects frames over 1 MiB and uses protocol version checks and correlation IDs.
- Production backend traffic must use certificate validation with no bypass switch.
- Plain HTTP backend access is permitted only when `EM_BACKEND_ALLOW_HTTP=true`; production packaging must always set it to `false`.
- Report vulnerabilities privately to the system owner rather than opening a public issue.
