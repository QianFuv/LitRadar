FROM rust:1.86-bookworm AS build

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates crates

RUN cargo build --release --bin api --bin ps-api --bin ps-cli --bin index --bin notify --bin push


FROM debian:bookworm-slim

WORKDIR /app

COPY --from=build /app/target/release/api /usr/local/bin/api
COPY --from=build /app/target/release/index /usr/local/bin/index
COPY --from=build /app/target/release/notify /usr/local/bin/notify
COPY --from=build /app/target/release/push /usr/local/bin/push
COPY --from=build /app/target/release/ps-api /usr/local/bin/ps-api
COPY --from=build /app/target/release/ps-cli /usr/local/bin/ps-cli

COPY libs/simple-linux libs/simple-linux
COPY data/meta data/meta

ENV API_HOST="0.0.0.0"
ENV PAPER_SCANNER_PROJECT_ROOT="/app"
ENV SIMPLE_TOKENIZER_PATH="/app/libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so"

EXPOSE 8000

CMD ["api"]
