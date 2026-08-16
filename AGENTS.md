# BRPX Repository Guide

## Scope

This repository contains the BRPX Rust server, its embedded admin UI, Linux lifecycle scripts, and operator documentation. Keep changes scoped to the requested feature. Do not rewrite unrelated response normalization or upstream compatibility logic.

## Project Map

- `src/main.rs`: process startup, Actix route registration, background workers, TLS listeners.
- `src/mods/management.rs`: runtime configuration, admin authentication, management API.
- `src/mods/access_control.rs`: local allow/deny rule validation, persistence, and matching.
- `src/mods/audit.rs`: sanitized audit queue, persistence, queries, episode enrichment.
- `src/mods/handler.rs`: public parsing API handlers and request context updates.
- `src/mods/upstream_res.rs`: Bilibili upstream requests and response normalization.
- `src/html/admin.html`: embedded admin application; it has no separate build step.
- `config.example.json` and `config.example.yml`: canonical versioned examples.
- `install.sh` and `uninstall.sh`: Linux/systemd lifecycle scripts.
- `docs/`: operator documentation.

## Required Commands

Run these from the repository root before handing off code:

```bash
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets
```

For shell changes, also run:

```bash
shellcheck install.sh uninstall.sh
bash -n install.sh uninstall.sh
```

## Configuration Rules

- Increment `config_version` when the persisted schema changes.
- Add a serde default for every new field so existing installations remain readable.
- Update both example files and `docs/configuration.md` in the same change.
- Migrations must preserve valid current fields, validate the result, and be covered by tests.
- Mark listener, worker, Redis, TLS, and rate-limit changes as restart-required.
- Never expose stored secret values through the management API. Keep the `********` round-trip behavior tested.

## Security Rules

- Never log raw access keys, cookies, authorization headers, signatures, passwords, private keys, refresh tokens, or signed URLs. Audit and management storage must not persist them; operational Redis caches are the only compatibility exception.
- Only trust forwarded IP headers from configured trusted proxy IPs or CIDRs.
- Admin mutations require both an authenticated session and the CSRF header.
- Access-token rules store only SHA-256 fingerprints.
- Destructive filesystem operations must target the fixed BRPX install/state directories and require explicit purge intent.

## Testing Expectations

- Rule changes need precedence, scope, expiry, and normalization tests.
- Management changes need unauthorized, CSRF, secret-redaction, and validation tests.
- Public route changes need exact route-to-upstream tests using fixtures or mocks, not live-network assertions.
- Audit changes need redaction, filtering, retention, and queue failure tests.
- Installer changes must remain idempotent and preserve existing configuration by default.

## Git Workflow

- Branches use the `codex/` prefix for Codex work.
- Use focused Conventional Commit messages such as `feat(audit): add sanitized request records`.
- Keep generated runtime data, local toolchains, credentials, certificates, and personal editor files out of Git.
- Track `Cargo.lock`; BRPX is an application and releases must be reproducible.
- Do not commit or modify the user-provided root `test.py` unless its ownership and final location are explicitly decided.
- Do not use destructive Git commands or rewrite existing user commits.
