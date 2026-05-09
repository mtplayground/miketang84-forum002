# miketang84-forum002

Rust forum application built with Axum, Askama, sqlx, PostgreSQL, and Tailwind CSS.

## Requirements

- Rust toolchain
- PostgreSQL
- `DATABASE_URL` pointing at a PostgreSQL database

The app loads environment variables from `.env` and then `.env.production` if those files exist.

## Local development

1. Copy the example env file if needed:

```bash
cp .env.example .env
```

2. Export the required runtime variables or edit `.env`:

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
export BIND_ADDR=0.0.0.0:8080
export SESSION_SECRET=dev-only-session-secret
export EDIT_WINDOW_MINUTES=15
export RUST_LOG=miketang84_forum002=debug,tower_http=info
```

3. Build the server:

```bash
cargo build
```

4. Run the server:

```bash
cargo run
```

The app listens on `http://0.0.0.0:8080` when `BIND_ADDR=0.0.0.0:8080`.

## Tailwind workflow

Build the stylesheet once:

```bash
./scripts/build-tailwind.sh
```

For a local edit/watch loop:

```bash
./scripts/build-tailwind.sh
./target/tailwind/tailwindcss \
  --config ./tailwind.config.js \
  --input ./assets/css/app.css \
  --output ./static/css/app.css \
  --watch
```

Run that watch process in one terminal and `cargo run` in another.

## Database migrations

The app runs embedded migrations automatically at startup, but the explicit migration command is:

```bash
DATABASE_URL=$(cat /workspace/.database_url) sqlx migrate run
```

That command requires `sqlx-cli` to be available in your local environment.

## Admin bootstrap and sample data

Bootstrap an admin account plus starter categories and sample threads with:

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
export SEED_ADMIN_USERNAME=admin
export SEED_ADMIN_PASSWORD=change-me-now
cargo run --bin seed
```

Optional seed metadata:

- `SEED_ADMIN_DISPLAY_NAME`
- `SEED_ADMIN_BIO`

The seed binary will:

- run pending migrations
- upsert the admin user and force its role to `admin`
- create or update the starter categories
- create sample threads and one sample reply per thread if they do not already exist

## Docker Compose

Start the full local stack:

```bash
docker compose up --build
```

Services:

- `app` on `http://localhost:8080`
- `postgres` on `localhost:5432`

Compose defaults:

- `DATABASE_URL=postgresql://forum:forum@postgres:5432/forum002`
- `BIND_ADDR=0.0.0.0:8080`
- `SESSION_SECRET=local-development-secret`
- `EDIT_WINDOW_MINUTES=15`
- `RUST_LOG=miketang84_forum002=debug,tower_http=info`

Stop the stack:

```bash
docker compose down
```

Stop the stack and delete the PostgreSQL volume:

```bash
docker compose down --volumes
```

## Environment variable reference

- `DATABASE_URL`: PostgreSQL connection string used by the app and seed binary
- `BIND_ADDR`: HTTP bind address such as `0.0.0.0:8080`
- `SESSION_SECRET`: secret used to sign session cookies
- `EDIT_WINDOW_MINUTES`: allowed self-edit/delete window for posts
- `RUST_LOG`: tracing filter override
- `SEED_ADMIN_USERNAME`: admin username for `cargo run --bin seed`
- `SEED_ADMIN_PASSWORD`: admin password for `cargo run --bin seed`
- `SEED_ADMIN_DISPLAY_NAME`: optional seeded admin display name
- `SEED_ADMIN_BIO`: optional seeded admin bio

## Deployment runbook

1. Set production environment variables, especially `DATABASE_URL`, `BIND_ADDR`, `SESSION_SECRET`, and `EDIT_WINDOW_MINUTES`.
2. Build the container image with the provided `Dockerfile`, or build the Rust binary with `cargo build --release`.
3. Run migrations before or during deploy.
   The app and the seed binary both call embedded migrations on startup.
4. Verify `/health` after the deploy.
5. If bootstrapping a fresh environment, run `cargo run --bin seed` once with the desired `SEED_ADMIN_*` values.

## Minimal production checklist

- Rotate `SESSION_SECRET` from development defaults before exposing the app publicly.
- Back up the PostgreSQL database before schema changes and on a recurring schedule.
- Confirm `DATABASE_URL` points to PostgreSQL, not a local or disposable database.
- Verify the seeded admin password is changed from any bootstrap default.
- Monitor application logs and `/health` during deploys and restarts.
