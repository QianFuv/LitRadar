# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM --platform=$BUILDPLATFORM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS node-toolchain

ARG BUILDARCH
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl gzip libatomic1 libstdc++6 xz-utils \
    && rm -rf /var/lib/apt/lists/* \
    && case "$BUILDARCH" in \
        amd64) node_arch=x64; archive_sha256=3883bfc73f9a680ca4eab04b196068aaaab1373ffa77d8fc1a4408222495b651 ;; \
        arm64) node_arch=arm64; archive_sha256=0945e2cde6aa0f54d980f03874dfe5cf3a549ce714c7fb9019e591ce0accf282 ;; \
        *) exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --max-time 300 \
        "https://nodejs.org/dist/v26.11.1/node-v26.11.1-linux-$node_arch.tar.xz" --output /tmp/node.tar.xz \
    && printf '%s  /tmp/node.tar.xz\n' "$archive_sha256" | sha256sum --check --strict \
    && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && npm install --global pnpm@12.10.1 --ignore-scripts --no-audit --no-fund \
    && node /usr/local/lib/node_modules/pnpm/install.js \
    && test "$(node --version)" = v26.11.1 \
    && test "$(pnpm --version)" = 12.10.1


FROM node-toolchain AS frontend-deps

WORKDIR /app

COPY app/package.json app/pnpm-lock.yaml app/pnpm-workspace.yaml ./

RUN --mount=type=cache,id=litradar-pnpm,target=/pnpm/store \
    pnpm install --frozen-lockfile --store-dir /pnpm/store


FROM node-toolchain AS frontend-build

WORKDIR /app

COPY --from=frontend-deps /app/node_modules node_modules/
COPY app/ ./
COPY scripts/generate-csp.mjs /scripts/generate-csp.mjs

RUN --mount=type=cache,id=litradar-next-build,target=/app/.next/cache \
    pnpm build
RUN find out -type f \( \
        -name '*.css' \
        -o -name '*.html' \
        -o -name '*.js' \
        -o -name '*.json' \
        -o -name '*.map' \
        -o -name '*.svg' \
        -o -name '*.txt' \
        -o -name '*.xml' \
    \) -exec gzip --best --keep --no-name {} +


FROM --platform=$BUILDPLATFORM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS go-build

WORKDIR /app

ARG TARGETARCH
ARG BUILDARCH
ENV CGO_ENABLED=1 GOTOOLCHAIN=local GOWORK=off GOENV=off GOFLAGS="" GOOS=linux GOARCH=$TARGETARCH

ENV PATH=/usr/local/go/bin:$PATH GOPATH=/go

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl gcc g++ git make cmake libatomic1 libstdc++6 \
    && rm -rf /var/lib/apt/lists/* \
    && case "$BUILDARCH" in \
        amd64) archive_sha256=ecbadb99091a3f46e31f5f934b068b1864eafa7995211b39eaddf76996045fe5 ;; \
        arm64) archive_sha256=94f3e30b8e374bc285e7dadc11e0865726b9bc6e85b841ccceaabc0214c6b7c8 ;; \
        *) exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --max-time 300 \
        "https://go.dev/dl/go1.27.2.linux-$BUILDARCH.tar.gz" --output /tmp/go.tar.gz \
    && printf '%s  /tmp/go.tar.gz\n' "$archive_sha256" | sha256sum --check --strict \
    && tar -xzf /tmp/go.tar.gz -C /usr/local \
    && rm /tmp/go.tar.gz \
    && test "$(go version | cut -d ' ' -f 3)" = go1.27.2

RUN if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
        case "$TARGETARCH" in \
            arm64) compiler="gcc-aarch64-linux-gnu g++-aarch64-linux-gnu"; headers=libc6-dev-arm64-cross ;; \
            amd64) compiler="gcc-x86-64-linux-gnu g++-x86-64-linux-gnu"; headers=libc6-dev-amd64-cross ;; \
            *) exit 1 ;; \
        esac; \
        apt-get update && apt-get install --yes --no-install-recommends $compiler "$headers" \
        && rm -rf /var/lib/apt/lists/*; \
    fi

COPY go.mod go.sum VERSION version.go ./
COPY third_party third_party
RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod go mod download && go mod verify
COPY cmd cmd
COPY internal internal
RUN rm -rf /app/internal/webassets/export
COPY --from=frontend-build /app/out internal/webassets/export
COPY assets assets
COPY scripts/go-build-inventory.sh /usr/local/bin/go-build-inventory
COPY --from=node-toolchain /usr/local/bin/node /usr/local/bin/node
COPY scripts/build-simple-tokenizer.mjs scripts/build-simple-tokenizer.mjs

RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod \
    --mount=type=cache,id=litradar-go-build-${TARGETARCH},target=/root/.cache/go-build \
    if [ "$TARGETARCH" = "$BUILDARCH" ]; then export CC=gcc CXX=g++; \
    elif [ "$TARGETARCH" = arm64 ]; then export CC=aarch64-linux-gnu-gcc CXX=aarch64-linux-gnu-g++; \
    elif [ "$TARGETARCH" = amd64 ]; then export CC=x86_64-linux-gnu-gcc CXX=x86_64-linux-gnu-g++; \
    else exit 1; fi \
    && mkdir -p /out \
    && node scripts/build-simple-tokenizer.mjs \
    && export CGO_CFLAGS="$(node --input-type=module -e 'import {simpleBuildEnvironment} from "./scripts/build-simple-tokenizer.mjs"; process.stdout.write(simpleBuildEnvironment().CGO_CFLAGS)')" \
    && go build -mod=readonly -trimpath -tags sqlite_fts5,sqlite_dbstat,litradar_web -o /out/litradar ./cmd/litradar \
    && sh /usr/local/bin/go-build-inventory


FROM --platform=$BUILDPLATFORM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS obscura-release

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


FROM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS runtime-base

WORKDIR /app

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl libgcc-s1 libstdc++6 passwd poppler-data poppler-utils \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 litradar \
    && useradd --uid 10001 --gid litradar --no-create-home --home-dir /app --shell /usr/sbin/nologin litradar \
    && mkdir -p /app/data \
    && chown -R litradar:litradar /app

COPY --from=obscura-release /out/obscura /out/obscura-worker /usr/local/bin/

COPY docs/third-party /usr/share/doc/litradar/third-party
COPY --from=go-build /out/inventory /usr/share/doc/litradar/third-party/go-inventory

RUN sha256sum /usr/bin/pdftotext /etc/ssl/certs/ca-certificates.crt \
    > /usr/share/doc/litradar/third-party/native.sha256 \
    && dpkg-query -W > /usr/share/doc/litradar/third-party/ubuntu-packages.txt

ENV HOME=/tmp \
    LITRADAR_OBSCURA_PATH=/usr/local/bin/obscura \
    LITRADAR_PDFTOTEXT_PATH=/usr/bin/pdftotext

USER 10001:10001


FROM runtime-base

COPY --from=go-build /out/litradar /usr/local/bin/litradar

COPY assets/meta /usr/share/litradar/meta

EXPOSE 8000

STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8000/health/ready >/dev/null || exit 1

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
