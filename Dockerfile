# syntax=docker/dockerfile:1

FROM node:24-alpine AS frontend-deps

WORKDIR /app

COPY app/package.json app/pnpm-lock.yaml ./

RUN --mount=type=cache,id=litradar-pnpm,target=/pnpm/store \
    corepack enable pnpm \
    && pnpm config set store-dir /pnpm/store \
    && pnpm install --frozen-lockfile


FROM node:24-alpine AS frontend-build

WORKDIR /app

COPY --from=frontend-deps /app/node_modules node_modules/
COPY app/ ./

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


FROM rust:1.96-bookworm AS rust-build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --locked --bin litradar


FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl passwd \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 litradar \
    && useradd --uid 10001 --gid litradar --no-create-home --home-dir /app --shell /usr/sbin/nologin litradar \
    && mkdir -p /app/data \
    && chown -R litradar:litradar /app

COPY --from=rust-build /app/target/release/litradar /usr/local/bin/litradar

COPY --chown=litradar:litradar libs/simple-linux libs/simple-linux
COPY --chown=litradar:litradar data/meta data/meta
COPY --chown=litradar:litradar --from=frontend-build /app/out web

ENV HOME=/tmp

USER litradar

EXPOSE 8000

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
