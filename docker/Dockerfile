# Stage 1: build
FROM rust:slim AS builder

WORKDIR /src
COPY . .

RUN cargo build --release --package nodewipe-cli

# Stage 2: runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/nodewipe /usr/local/bin/nodewipe

RUN useradd -m -u 1000 nodewipe
USER nodewipe

WORKDIR /scan

ENTRYPOINT ["nodewipe"]
CMD ["scan", "--json"]

