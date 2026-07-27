# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM node:24-alpine@sha256:a0b9bf06e4e6193cf7a0f58816cc935ff8c2a908f81e6f1a95432d679c54fbfd AS frontend-deps

WORKDIR /app

COPY app/package.json app/pnpm-lock.yaml ./

RUN --mount=type=cache,id=litradar-pnpm,target=/pnpm/store \
    corepack enable pnpm \
    && pnpm config set store-dir /pnpm/store \
    && pnpm install --frozen-lockfile


FROM node:24-alpine@sha256:a0b9bf06e4e6193cf7a0f58816cc935ff8c2a908f81e6f1a95432d679c54fbfd AS frontend-build

WORKDIR /app

COPY --from=frontend-deps /app/node_modules node_modules/
COPY app/ ./
COPY scripts/generate-csp.mjs /scripts/generate-csp.mjs
COPY testdata /testdata

RUN corepack enable pnpm && pnpm build
RUN apk add --no-cache gzip \
    && find out -type f \( \
        -name '*.css' \
        -o -name '*.html' \
        -o -name '*.js' \
        -o -name '*.json' \
        -o -name '*.map' \
        -o -name '*.svg' \
        -o -name '*.txt' \
        -o -name '*.xml' \
    \) -exec gzip --best --keep --no-name {} +


FROM rust:1.96-bookworm@sha256:a339861ae23e9abb272cea45dfafde21760d2ce6577a70f8a926153677902663 AS rust-build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN --mount=type=cache,id=litradar-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=litradar-cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=litradar-cargo-target,target=/app/target \
    cargo build --release --locked --bin litradar \
    && cp /app/target/release/litradar /app/litradar


FROM debian:trixie-slim@sha256:020c0d20b9880058cbe785a9db107156c3c75c2ac944a6aa7ab59f2add76a7bd

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libstdc++6 passwd \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 litradar \
    && useradd --uid 10001 --gid litradar --no-create-home --home-dir /app --shell /usr/sbin/nologin litradar \
    && mkdir -p /app/data \
    && chown -R litradar:litradar /app

COPY --from=rust-build /app/litradar /usr/local/bin/litradar

COPY --chown=litradar:litradar libs/simple-linux libs/simple-linux
COPY data/meta /usr/share/litradar/meta
COPY --chown=litradar:litradar --from=frontend-build /app/out web

ENV HOME=/tmp

USER 10001:10001

EXPOSE 8000

STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8000/health/ready >/dev/null || exit 1

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
