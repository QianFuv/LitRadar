FROM rust:1.96-bookworm AS build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --bin admin --bin api --bin ps-api --bin index --bin notify --bin push --bin scheduler --bin worker


FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl passwd \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 paper \
    && useradd --uid 10001 --gid paper --no-create-home --home-dir /app --shell /usr/sbin/nologin paper \
    && mkdir -p /app/data \
    && chown -R paper:paper /app

COPY --from=build /app/target/release/admin /usr/local/bin/admin
COPY --from=build /app/target/release/api /usr/local/bin/api
COPY --from=build /app/target/release/index /usr/local/bin/index
COPY --from=build /app/target/release/notify /usr/local/bin/notify
COPY --from=build /app/target/release/push /usr/local/bin/push
COPY --from=build /app/target/release/ps-api /usr/local/bin/ps-api
COPY --from=build /app/target/release/scheduler /usr/local/bin/scheduler
COPY --from=build /app/target/release/worker /usr/local/bin/worker

COPY --chown=paper:paper libs/simple-linux libs/simple-linux
COPY --chown=paper:paper data/meta data/meta

ENV HOME=/tmp

USER paper

EXPOSE 8000

CMD ["api", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app"]
