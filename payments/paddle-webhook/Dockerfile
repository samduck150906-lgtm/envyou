# Build context must be the REPO ROOT (the crate path-depends on
# ../../crates/envyou-core). In Railway: Root Directory = repo root,
# Dockerfile Path = payments/paddle-webhook/Dockerfile.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cd payments/paddle-webhook && cargo build --release
RUN cp /src/payments/paddle-webhook/target/release/paddle-webhook /usr/local/bin/paddle-webhook

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /usr/local/bin/paddle-webhook /usr/local/bin/paddle-webhook
ENV PORT=8080
EXPOSE 8080
CMD ["paddle-webhook"]
