# miketang84-forum002

Rust forum application scaffolded with Axum and Tokio.

## Requirements

- Rust toolchain
- PostgreSQL connection string provided through `DATABASE_URL`

## Run locally

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
export SESSION_SECRET=dev-only-session-secret
export EDIT_WINDOW_MINUTES=15
cargo build
BIND_ADDR=0.0.0.0:8080 cargo run
```

The server exposes:

- `GET /`
- `GET /health`

Environment variables:

- `DATABASE_URL`: PostgreSQL connection string
- `BIND_ADDR`: bind address for the HTTP server, for example `0.0.0.0:8080`
- `SESSION_SECRET`: secret used for session signing/encryption
- `EDIT_WINDOW_MINUTES`: post edit window length in minutes
- `RUST_LOG`: tracing filter, optional

Copy [`.env.example`](/workspace/.env.example) to `.env` for local development if preferred.
