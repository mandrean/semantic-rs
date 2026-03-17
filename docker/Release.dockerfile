FROM rust:latest AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /usr/src/semantic-rs
COPY . .
RUN cargo install --path .

FROM rust:slim
ENV RUST_LOG=info
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/semantic-rs /usr/local/bin/semantic-rs
WORKDIR /home
ENTRYPOINT ["/usr/local/bin/semantic-rs"]
