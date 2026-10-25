FROM node:24-trixie-slim AS frontend
RUN corepack enable
WORKDIR /build/frontend
COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm build

FROM rust:1.95.0-trixie AS backend
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /build
ARG TARGETARCH
ARG CARGO_REGISTRY_MIRROR=
RUN if [ -n "$CARGO_REGISTRY_MIRROR" ]; then mkdir -p .cargo && printf '[source.crates-io]\nreplace-with = "build-mirror"\n[source.build-mirror]\nregistry = "%s"\n' "$CARGO_REGISTRY_MIRROR" > .cargo/config.toml; fi
COPY Cargo.toml Cargo.lock ./
COPY backend/ backend/
RUN --mount=type=cache,id=website-cargo-${TARGETARCH},target=/usr/local/cargo/registry \
    --mount=type=cache,id=website-target-${TARGETARCH},target=/build/target \
    cargo build --locked --release && mkdir -p /out && cp target/release/veydeck-website-manager /out/website-manager

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl libssl3t64 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=backend /out/website-manager /app/website-manager
COPY --from=frontend /build/frontend/dist /app/frontend/dist
ENV RUST_LOG=info
EXPOSE 8080
CMD ["/app/website-manager"]
