FROM rust:latest as builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY . .
# Force rebuild by removing the dummy binary
RUN rm -f target/release/auth-rs target/release/deps/auth*
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 curl && rm -rf /var/lib/apt/lists/*
WORKDIR /app

COPY --from=builder /app/target/release/auth-rs /app/auth-rs
COPY --from=builder /app/db/migrations /app/migrations
COPY --from=builder /app/src/events/schemas /app/schemas

EXPOSE 8080
CMD ["/app/auth-rs"]
