FROM rust:1.85-slim AS builder
WORKDIR /app
COPY Cargo.toml ./
COPY src/ ./src/
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/social-feed /usr/local/bin/
COPY capability.yaml /capability.yaml
EXPOSE 7001
ENV PORT=7001
ENV DATA_DIR=/data
CMD ["social-feed"]
