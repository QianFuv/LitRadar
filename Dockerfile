FROM rust:1.86-bookworm AS build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --bin api --bin ps-api --bin index --bin notify --bin push --bin scheduler --bin worker


FROM debian:bookworm-slim

WORKDIR /app

COPY --from=build /app/target/release/api /usr/local/bin/api
COPY --from=build /app/target/release/index /usr/local/bin/index
COPY --from=build /app/target/release/notify /usr/local/bin/notify
COPY --from=build /app/target/release/push /usr/local/bin/push
COPY --from=build /app/target/release/ps-api /usr/local/bin/ps-api
COPY --from=build /app/target/release/scheduler /usr/local/bin/scheduler
COPY --from=build /app/target/release/worker /usr/local/bin/worker

COPY libs/simple-linux libs/simple-linux
COPY data/meta data/meta

EXPOSE 8000

CMD ["api", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app"]
