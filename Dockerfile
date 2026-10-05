# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM --platform=$BUILDPLATFORM node:24-alpine@sha256:a0b9bf06e4e6193cf7a0f58816cc935ff8c2a908f81e6f1a95432d679c54fbfd AS frontend-deps

WORKDIR /app

COPY app/package.json app/pnpm-lock.yaml ./

RUN --mount=type=cache,id=litradar-pnpm,target=/pnpm/store \
    corepack enable pnpm \
    && pnpm config set store-dir /pnpm/store \
    && pnpm install --frozen-lockfile


FROM --platform=$BUILDPLATFORM node:24-alpine@sha256:a0b9bf06e4e6193cf7a0f58816cc935ff8c2a908f81e6f1a95432d679c54fbfd AS frontend-build

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


FROM --platform=$BUILDPLATFORM golang:1.27.1-bookworm@sha256:69a7b9788769bec032d238959b61854e9ae87f57be9029ec04e9885fabf99195 AS go-build

WORKDIR /app

ARG TARGETARCH
ARG BUILDARCH
ENV CGO_ENABLED=1 GOTOOLCHAIN=local GOWORK=off GOENV=off GOFLAGS="" GOOS=linux GOARCH=$TARGETARCH

RUN if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
        case "$TARGETARCH" in \
            arm64) compiler=gcc-aarch64-linux-gnu; headers=libc6-dev-arm64-cross ;; \
            amd64) compiler=gcc-x86-64-linux-gnu; headers=libc6-dev-amd64-cross ;; \
            *) exit 1 ;; \
        esac; \
        apt-get update && apt-get install --yes --no-install-recommends "$compiler" "$headers" \
        && rm -rf /var/lib/apt/lists/*; \
    fi

COPY go.mod go.sum VERSION version.go ./
COPY third_party third_party
RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod go mod download && go mod verify
COPY cmd cmd
COPY internal internal
COPY assets assets
COPY scripts/go-build-inventory.sh /usr/local/bin/go-build-inventory

RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod \
    --mount=type=cache,id=litradar-go-build-${TARGETARCH},target=/root/.cache/go-build \
    if [ "$TARGETARCH" = "$BUILDARCH" ]; then export CC=gcc; \
    elif [ "$TARGETARCH" = arm64 ]; then export CC=aarch64-linux-gnu-gcc; \
    elif [ "$TARGETARCH" = amd64 ]; then export CC=x86_64-linux-gnu-gcc; \
    else exit 1; fi \
    && mkdir -p /out \
    && go build -mod=readonly -trimpath -tags sqlite_fts5,sqlite_dbstat -o /out/litradar ./cmd/litradar \
    && sh /usr/local/bin/go-build-inventory


FROM --platform=$BUILDPLATFORM debian:trixie-slim@sha256:020c0d20b9880058cbe785a9db107156c3c75c2ac944a6aa7ab59f2add76a7bd AS obscura-release

ARG TARGETARCH
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
RUN case "$TARGETARCH" in \
        amd64) asset=obscura-x86_64-linux-stealth.tar.gz; archive_sha256=49b53f74a509764c42a35c8e73a37301399e1500d43564fad4cb641721efc753 ;; \
        arm64) asset=obscura-aarch64-linux-stealth.tar.gz; archive_sha256=56eacea66e4a5b0ab0f39343183c308816b426f604259b9cc41402488bf998f7 ;; \
        *) exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --max-time 300 \
        "https://github.com/h4ckf0r0day/obscura/releases/download/v0.2.4/$asset" \
        --output /tmp/obscura.tar.gz \
    && printf '%s  /tmp/obscura.tar.gz\n' "$archive_sha256" | sha256sum --check --strict \
    && mkdir /out \
    && tar -xzf /tmp/obscura.tar.gz -C /out obscura obscura-worker \
    && chmod 755 /out/obscura /out/obscura-worker


FROM golang:1.27.1-bookworm@sha256:69a7b9788769bec032d238959b61854e9ae87f57be9029ec04e9885fabf99195 AS simple-tokenizer-build

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

COPY --from=obscura-release /out/obscura /out/obscura-worker /usr/local/bin/
COPY --from=simple-tokenizer-build /simple/output/libsimple.so /usr/lib/litradar/libsimple.so

COPY docs/third-party /usr/share/doc/litradar/third-party
COPY --from=go-build /out/inventory /usr/share/doc/litradar/third-party/go-inventory

RUN sha256sum /usr/lib/litradar/libsimple.so /usr/bin/pdftotext /etc/ssl/certs/ca-certificates.crt \
    > /usr/share/doc/litradar/third-party/native.sha256 \
    && dpkg-query -W > /usr/share/doc/litradar/third-party/debian-packages.txt

ENV HOME=/tmp \
    LITRADAR_OBSCURA_PATH=/usr/local/bin/obscura \
    LITRADAR_PDFTOTEXT_PATH=/usr/bin/pdftotext

USER 10001:10001


FROM runtime-base

COPY --from=go-build /out/litradar /usr/local/bin/litradar

COPY assets/meta /usr/share/litradar/meta
COPY --chown=litradar:litradar --from=frontend-build /app/out web

EXPOSE 8000

STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8000/health/ready >/dev/null || exit 1

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
