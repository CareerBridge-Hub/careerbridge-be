# CareerBridge backend

Actix Web API for the CareerBridge capstone: Supabase authentication and PostgreSQL, private temporary CV storage, local 384-dimensional E5 embeddings, and Gemini through an OpenAI-compatible provider interface. No chatbot.

Start with [Supabase and local setup](docs/SETUP.md), the [API contract](docs/openapi.json), and [validation results](docs/VALIDATION.md).

Docker targets are available for the API (`runtime`, default), migrations (`migrate`), and credential-free tests (`test`). See [container setup](docs/SETUP.md#docker) for build/run commands and the persistent model cache.

```sh
cp .env.example .env
# Configure credentials, then run migrations with the schema-owner connection.
cargo run --bin migrate
cargo run --release --bin careerbridge-be
```

`GET /health` is a liveness check; `/ready` also checks PostgreSQL. Model initialization must finish before the HTTP listener starts. HTTP defaults to `127.0.0.1:8080`; frontend development uses port 5173.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo audit
```

Database integration tests require a separate disposable database; see the setup guide. The browser journey uses a real API/database/model and fixture authentication/storage/LLM services. Real Supabase/Gemini smoke checks remain distinct.

The catalog contains **66 skills, 9 occupations, 12 institutions, and 40 sample training programs** exported from the existing frontend. It creates no demo passwords or real application accounts. Sample programs are not verified real enrollment opportunities.

Domain rules and DTOs live in `domain`, provider ports in `application`, adapters/workers in `infrastructure`, and versioned HTTP routes in `presentation`. CV parsing executes in a bounded subprocess of the backend binary; embedding inference executes on one shared, bounded CPU worker. SQL migrations are embedded into the migration binary; `build.rs` ensures new migrations trigger recompilation.
