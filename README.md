# miketang84-forum002

Rust forum application scaffolded with Axum and Tokio.

## Requirements

- Rust toolchain
- PostgreSQL connection string provided through `DATABASE_URL`

## Run locally

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
cargo build
HOST=0.0.0.0 PORT=8080 cargo run
```

The server exposes:

- `GET /`
- `GET /health`

Environment variables:

- `HOST`: bind host, defaults to `0.0.0.0`
- `PORT`: bind port, defaults to `8080`
- `RUST_LOG`: tracing filter, optional
