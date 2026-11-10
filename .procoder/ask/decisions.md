## Docker compose FE+BE on one VPS: HTTPS / domain setup?

- Host already has reverse proxy (nginx/Caddy) with TLS: compose exposes FE only on 127.0.0.1:3000
- Add Caddy to compose: auto HTTPS for a domain, ports 80/443
- No domain yet: FE on port 80 plain HTTP (AUTH_COOKIE_SECURE=false, testing only)

**Answer (2026-11-11, user):** Add Caddy to compose.

## Where does the compose file live?

- BE repo deploy/compose.yaml, BE workflow syncs it to VPS; FE workflow only bumps FE image
- New separate infra repo

**Answer (2026-11-11, user):** BE repo deploy/compose.yaml.
