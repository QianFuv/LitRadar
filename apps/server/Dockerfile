# syntax=docker/dockerfile:1@sha256:87999aa3d42bdc6bea60565083ee17e86d1f3339802f543c0d03998580f9cb89

FROM --platform=$BUILDPLATFORM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS go-build

WORKDIR /app

ARG TARGETARCH
ARG BUILDARCH
ENV CGO_ENABLED=1 GOTOOLCHAIN=local GOWORK=off GOENV=off GOFLAGS="" GOOS=linux GOARCH=$TARGETARCH

ENV PATH=/usr/local/go/bin:$PATH GOPATH=/go

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates=20260816ubuntu0.26.04.2 curl gcc git \
    && test "$(dpkg-query -W -f='${Version}' ca-certificates)" = 20260816ubuntu0.26.04.2 \
    && rm -rf /var/lib/apt/lists/* \
    && case "$BUILDARCH" in \
        amd64) archive_sha256=65efbc80fdc1fa2cd2631cad5b481777a68e8725f34e89f50fc6ac714e379b40 ;; \
        arm64) archive_sha256=a06e99309dbcbdca1a8ccd2282c3ee146e55930886e26b4f1193269587165f49 ;; \
        *) exit 1 ;; \
    esac \
    && curl --fail --location --retry 3 --max-time 300 \
        "https://go.dev/dl/go1.27.3.linux-$BUILDARCH.tar.gz" --output /tmp/go.tar.gz \
    && printf '%s  /tmp/go.tar.gz\n' "$archive_sha256" | sha256sum --check --strict \
    && tar -xzf /tmp/go.tar.gz -C /usr/local \
    && rm /tmp/go.tar.gz \
    && test "$(go version | cut -d ' ' -f 3)" = go1.27.3

RUN if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
        case "$TARGETARCH" in \
            arm64) compiler=gcc-aarch64-linux-gnu; headers=libc6-dev-arm64-cross ;; \
            amd64) compiler=gcc-x86-64-linux-gnu; headers=libc6-dev-amd64-cross ;; \
            *) exit 1 ;; \
        esac; \
        apt-get update && apt-get install --yes --no-install-recommends $compiler "$headers" \
        && rm -rf /var/lib/apt/lists/*; \
    fi

COPY go.mod go.sum VERSION version.go ./
COPY third_party third_party
RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod go mod download && go mod verify
COPY apps/server apps/server
COPY apps/server/scripts/go-build-inventory.sh /usr/local/bin/go-build-inventory

RUN --mount=type=cache,id=litradar-go-mod,target=/go/pkg/mod \
    --mount=type=cache,id=litradar-go-build-${TARGETARCH},target=/root/.cache/go-build \
    if [ "$TARGETARCH" = "$BUILDARCH" ]; then export CC=gcc; \
    elif [ "$TARGETARCH" = arm64 ]; then export CC=aarch64-linux-gnu-gcc; \
    elif [ "$TARGETARCH" = amd64 ]; then export CC=x86_64-linux-gnu-gcc; \
    else exit 1; fi \
    && mkdir -p /out \
    && go build -mod=readonly -trimpath -tags sqlite_fts5,sqlite_dbstat -o /out/litradar ./apps/server \
    && sh /usr/local/bin/go-build-inventory


FROM --platform=$BUILDPLATFORM ubuntu:26.04@sha256:f144425ff09be612d6d9ad965196e9cdc23dae1f42110a8a11a3e9a8198759f7 AS obscura-release

ARG TARGETARCH
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates=20260816ubuntu0.26.04.2 curl \
    && test "$(dpkg-query -W -f='${Version}' ca-certificates)" = 20260816ubuntu0.26.04.2 \
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
    && apt-get install --yes --no-install-recommends ca-certificates=20260816ubuntu0.26.04.2 curl libgcc-s1 libstdc++6 passwd poppler-data poppler-utils \
    && test "$(dpkg-query -W -f='${Version}' ca-certificates)" = 20260816ubuntu0.26.04.2 \
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

COPY apps/server/assets/meta /usr/share/litradar/meta

EXPOSE 8000

STOPSIGNAL SIGTERM

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8000/health/ready >/dev/null || exit 1

ENTRYPOINT ["litradar"]

CMD ["serve", "--host", "0.0.0.0", "--port", "8000", "--project-root", "/app", "--secret-key-file", "/run/secrets/litradar_key"]
