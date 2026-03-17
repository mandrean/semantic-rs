FROM rust:slim
ARG TARGETARCH
COPY ${TARGETARCH}/semantic-rs /usr/local/bin/semantic-rs
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
ENV RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/semantic-rs"]
