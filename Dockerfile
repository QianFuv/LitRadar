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
COPY tests/data /tests/data

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


FROM rust:1.96-bookworm@sha256:a339861ae23e9abb272cea45dfafde21760d2ce6577a70f8a926153677902663 AS obscura-build

RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake clang fonts-dejavu-core fonts-liberation libclang-dev llvm-dev python3 \
    && rm -rf /var/lib/apt/lists/*

ADD --checksum=sha256:92e742e3c1f4d030561b0df559c4a0a5707b3f3c977bee1307c38d988404003c \
    https://codeload.github.com/h4ckf0r0day/obscura/tar.gz/a1e09de68c7617b8079fbb1661b0548c501971c1 /tmp/obscura.tar.gz
COPY docs/third-party/obscura-rustls.patch /tmp/obscura-rustls.patch

ARG TARGETARCH

RUN case "$TARGETARCH" in \
        amd64) v8_target=x86_64-unknown-linux-gnu; v8_sha256=a2681fde53018abd3366018f132617dd598002df78f156c7bc6d2ff608b56567 ;; \
        arm64) v8_target=aarch64-unknown-linux-gnu; v8_sha256=e368d0cb41c179c43a990e4ec4a55e44dd9762f1d34be15696acc4433f8a6c92 ;; \
        *) exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --max-time 120 \
        "https://github.com/denoland/rusty_v8/releases/download/v137.3.0/librusty_v8_release_${v8_target}.a.gz" \
        --output /tmp/librusty_v8.a.gz \
    && printf '%s  /tmp/librusty_v8.a.gz\n' "$v8_sha256" | sha256sum --check --strict

WORKDIR /obscura

RUN tar -xzf /tmp/obscura.tar.gz --strip-components=1 \
    && patch --fuzz=0 --strip=1 < /tmp/obscura-rustls.patch \
    && rm /tmp/obscura.tar.gz /tmp/obscura-rustls.patch

RUN --mount=type=cache,id=litradar-obscura-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=litradar-obscura-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=litradar-obscura-target,target=/obscura/target \
    OBSCURA_VERSION=0.2.2+litradar.1 RUSTY_V8_ARCHIVE=/tmp/librusty_v8.a.gz \
    CARGO_BUILD_JOBS=2 CARGO_PROFILE_RELEASE_STRIP=symbols \
    cargo build --release --locked -p obscura-cli --bin obscura --no-default-features --features render,stealth \
    && cp target/release/obscura /usr/local/bin/obscura \
    && cargo tree --locked -p obscura-cli --no-default-features --features render,stealth --edges normal --prefix none \
        > /obscura/dependencies.txt \
    && mkdir /obscura/licenses \
    && cp /usr/share/doc/fonts-dejavu-core/copyright /obscura/licenses/DejaVu-copyright \
    && cp /usr/share/doc/fonts-liberation/copyright /obscura/licenses/Liberation-copyright \
    && cp crates/obscura-render/assets/LICENSE-NOTO-COLOR-EMOJI.txt /obscura/licenses/ \
    && cp crates/obscura-render/assets/FONT-PROVENANCE.md /obscura/licenses/ \
    && find vendor -type f \( -iname 'license*' -o -iname 'notice*' -o -iname 'copying*' \) \
        -exec cp --parents --target-directory=/obscura/licenses {} + \
    && cd /usr/local/cargo/registry/src \
    && find . -type f \( -iname 'license*' -o -iname 'notice*' -o -iname 'copying*' \) \
        -exec cp --parents --target-directory=/obscura/licenses {} +


FROM rust:1.96-bookworm@sha256:a339861ae23e9abb272cea45dfafde21760d2ce6577a70f8a926153677902663 AS simple-tokenizer-build

RUN apt-get update \
    && apt-get install --yes --no-install-recommends cmake g++ \
    && rm -rf /var/lib/apt/lists/*

ADD --checksum=sha256:d60f39ecad1f4fcf46485810708353777224ddc3829b7c9de865034277481e61 \
    https://codeload.github.com/wangfenjin/simple/tar.gz/45db071ba8043ffe8a2e5dfe41f9d68fb477576c /tmp/simple.tar.gz

RUN mkdir /simple \
    && tar -xzf /tmp/simple.tar.gz -C /simple --strip-components=1 \
    && cmake -S /simple -B /simple/build -DCMAKE_BUILD_TYPE=Release \
        -DSIMPLE_WITH_JIEBA=OFF -DBUILD_SQLITE3=OFF -DBUILD_TEST_EXAMPLE=OFF \
        -DBUILD_STATIC=OFF -DCMAKE_LIBRARY_OUTPUT_DIRECTORY=/simple/output \
    && cmake --build /simple/build --target simple --parallel 2


FROM debian:trixie-slim@sha256:020c0d20b9880058cbe785a9db107156c3c75c2ac944a6aa7ab59f2add76a7bd AS runtime-base

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libgcc-s1 libstdc++6 passwd poppler-data poppler-utils \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 litradar \
    && useradd --uid 10001 --gid litradar --no-create-home --home-dir /app --shell /usr/sbin/nologin litradar \
    && mkdir -p /app/data \
    && chown -R litradar:litradar /app

COPY --from=obscura-build /usr/local/bin/obscura /usr/local/bin/obscura
COPY --from=simple-tokenizer-build /simple/output/libsimple.so /usr/lib/litradar/libsimple.so

COPY docs/third-party /usr/share/doc/litradar/third-party
COPY --from=obscura-build /obscura/licenses /usr/share/doc/litradar/third-party/obscura-dependencies
COPY --from=obscura-build /obscura/dependencies.txt /usr/share/doc/litradar/third-party/Obscura-dependencies.txt

ENV HOME=/tmp \
    LITRADAR_OBSCURA_PATH=/usr/local/bin/obscura \
    LITRADAR_PDFTOTEXT_PATH=/usr/bin/pdftotext

USER 10001:10001


FROM runtime-base

COPY --from=rust-build /app/litradar /usr/local/bin/litradar

COPY data/meta /usr/share/litradar/meta
COPY --chown=litradar:litradar --from=frontend-build /app/out web

EXPOSE 8000

STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8000/health/ready >/dev/null || exit 1

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
