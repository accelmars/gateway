# Stage 1: Build — static musl binary
FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

# Stage 2: Run — scratch image, ~8MB
FROM scratch
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/gateway /gateway
COPY --from=builder /app/gateway.example.toml /gateway.toml
EXPOSE 8080
ENTRYPOINT ["/gateway", "serve", "--config", "/gateway.toml"]
