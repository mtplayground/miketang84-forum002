FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin miketang84-forum002

FROM debian:bookworm-slim AS runtime
WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 appuser

COPY --from=builder /app/target/release/miketang84-forum002 /usr/local/bin/miketang84-forum002
COPY --from=builder /app/static /app/static

ENV BIND_ADDR=0.0.0.0:8080
EXPOSE 8080

USER appuser

ENTRYPOINT ["/usr/local/bin/miketang84-forum002"]
