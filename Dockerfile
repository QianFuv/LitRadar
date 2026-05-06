FROM python:3.12-slim-trixie AS build

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

WORKDIR /app

COPY pyproject.toml uv.lock README.md ./

RUN uv sync --frozen --no-dev --no-install-project

COPY scripts/ scripts/

RUN uv sync --frozen --no-dev


FROM python:3.12-slim-trixie

WORKDIR /app

COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/
COPY --from=build /app/.venv .venv/
COPY --from=build /app/pyproject.toml ./
COPY --from=build /app/uv.lock ./
COPY --from=build /app/README.md ./
COPY --from=build /app/scripts scripts/

COPY libs/simple-linux libs/simple-linux
COPY data/meta data/meta

ENV PATH="/app/.venv/bin:$PATH"
ENV API_HOST="0.0.0.0"
ENV SIMPLE_TOKENIZER_PATH="/app/libs/simple-linux/libsimple-linux-ubuntu-latest/libsimple.so"

EXPOSE 8000

CMD ["api"]
