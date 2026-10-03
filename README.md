# CareerBridge backend

Rust HTTP API built with Actix Web.

## Run

```sh
cargo run
```

The server listens on `127.0.0.1:8080` by default. Set `HOST` or `PORT` to override it. `GET /health` returns `{"status":"ok"}`.

## Architecture

- `domain`: business rules and types; no framework or database dependencies.
- `application`: use cases; coordinates domain behavior and ports.
- `infrastructure`: adapters and runtime configuration.
- `presentation/http`: Actix routes and HTTP request/response mapping.

Keep dependencies pointing inward: HTTP and infrastructure code may call application use cases, and application code may depend on domain types. Add domain models and use cases when the product behavior is defined; the health endpoint is only a transport-level check.
