FROM rust:1.95-slim AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p mizan-api

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/mizan-api /usr/local/bin/mizan-api

EXPOSE 8080
CMD ["mizan-api"]
