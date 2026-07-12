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

RUN cargo build --release --bin admin --bin api --bin litradar-api --bin index --bin notify --bin push --bin scheduler --bin worker


FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl passwd \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 litradar \
    && useradd --uid 10001 --gid litradar --no-create-home --home-dir /app --shell /usr/sbin/nologin litradar \
    && mkdir -p /app/data \
    && chown -R litradar:litradar /app

COPY --from=rust-build /app/target/release/admin /usr/local/bin/admin
COPY --from=rust-build /app/target/release/api /usr/local/bin/api
COPY --from=rust-build /app/target/release/index /usr/local/bin/index
COPY --from=rust-build /app/target/release/notify /usr/local/bin/notify
COPY --from=rust-build /app/target/release/push /usr/local/bin/push
COPY --from=rust-build /app/target/release/litradar-api /usr/local/bin/litradar-api
COPY --from=rust-build /app/target/release/scheduler /usr/local/bin/scheduler
COPY --from=rust-build /app/target/release/worker /usr/local/bin/worker

COPY --chown=litradar:litradar libs/simple-linux libs/simple-linux
COPY --chown=litradar:litradar data/meta data/meta
COPY --chown=litradar:litradar --from=frontend-build /app/out web

ENV HOME=/tmp

USER litradar

EXPOSE 8000

CMD ["api", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app"]
