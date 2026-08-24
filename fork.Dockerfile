# syntax=docker/dockerfile:1

## Personal fork image build for komodo core / periphery / cli.
## Consumed by docker-bake.hcl in YewFence/actions (target per stage below).
## Upstream Dockerfiles under bin/ and ui/ stay untouched; this file is fork-only.

############################################################
## Toolchain: debian trixie + mise (rust / node / aube from repo mise.toml)
############################################################
FROM debian:trixie-slim AS toolchain

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl git ca-certificates build-essential pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

ENV MISE_DATA_DIR="/mise"
ENV MISE_INSTALL_PATH="/usr/local/bin/mise"
ENV PATH="/mise/shims:$PATH"

# Pin mise for reproducible builds; bump deliberately.
RUN curl https://mise.run | MISE_VERSION="v2026.8.12" MISE_INSTALL_PATH="/usr/local/bin/mise" sh

WORKDIR /build
COPY mise.toml ./

# Optional GitHub token secret avoids unauthenticated API rate limits
# when mise resolves the aube GitHub release.
RUN --mount=type=secret,id=GITHUB_TOKEN,required=false \
    export GITHUB_TOKEN="$(cat /run/secrets/GITHUB_TOKEN 2>/dev/null || true)"; \
    mise trust mise.toml && mise install

############################################################
## Rust binaries: core + periphery + km (cli), built once, stripped
############################################################
FROM toolchain AS rust-builder

COPY Cargo.toml Cargo.lock ./
COPY ./lib ./lib
COPY ./client/core/rs ./client/core/rs
COPY ./client/periphery ./client/periphery
COPY ./bin/core ./bin/core
COPY ./bin/periphery ./bin/periphery
COPY ./bin/cli ./bin/cli
COPY ./xtask ./xtask

# Cache mounts pair with cache-from/to=type=gha (mode=max) in the bake workflow.
RUN --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build -p komodo_core -p komodo_periphery -p komodo_cli --release && \
    mkdir /out && \
    cp target/release/core target/release/periphery target/release/km /out/ && \
    strip /out/core /out/periphery /out/km

############################################################
## UI static bundle via aube workspace
############################################################
FROM toolchain AS ui-builder

# Avoid OOM during vite build (see .devcontainer).
ENV NODE_OPTIONS="--max-old-space-size=4096"

COPY aube-workspace.yaml ./
COPY ./ui ./ui
COPY ./client/core/ts ./client/core/ts

# Optionally bake in a specific Komodo host for the ui.
ARG VITE_KOMODO_HOST=""
ENV VITE_KOMODO_HOST=$VITE_KOMODO_HOST

RUN aube install && \
    cd client/core/ts && aube build && \
    cd ../../../ui && aube build

############################################################
## Komodo Core (includes km cli + ui bundle + deno for actions)
############################################################
FROM debian:trixie-slim AS core

COPY ./bin/core/starship.toml /starship.toml
COPY ./bin/core/debian-deps.sh .
RUN sh ./debian-deps.sh && rm ./debian-deps.sh

WORKDIR /app

COPY ./config/core.config.toml /config/.default.config.toml
COPY --from=ui-builder /build/ui/dist /app/ui
COPY --from=rust-builder /out/core /usr/local/bin/core
COPY --from=rust-builder /out/km /usr/local/bin/km
COPY --from=denoland/deno:bin /deno /usr/local/bin/deno

# Set $DENO_DIR and preload external Deno deps
ENV DENO_DIR=/action-cache/deno
RUN mkdir /action-cache && \
    cd /action-cache && \
    deno install jsr:@std/yaml jsr:@std/toml

COPY ./bin/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

EXPOSE 9120

ENV KOMODO_CLI_CONFIG_PATHS="/config"
# This ensures any `komodo.cli.*` takes precedence over the Core `/config/*config.*`
ENV KOMODO_CLI_CONFIG_KEYWORDS="*config.*,*komodo.cli*.*"

ENTRYPOINT [ "entrypoint.sh" ]
CMD [ "core" ]

# Label to prevent Komodo from stopping with StopAllContainers
LABEL komodo.skip="true"

############################################################
## Komodo Periphery
############################################################
FROM debian:trixie-slim AS periphery

COPY ./bin/periphery/starship.toml /starship.toml
COPY ./bin/periphery/debian-deps.sh .
RUN sh ./debian-deps.sh && rm ./debian-deps.sh

COPY --from=rust-builder /out/periphery /usr/local/bin/periphery

COPY ./bin/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

EXPOSE 8120

# Can mount config file to /config/*config*.toml and it will be picked up.
ENV PERIPHERY_CONFIG_PATHS="/config"
# Change the default in container to /config/keys to match Core
ENV PERIPHERY_PRIVATE_KEY="file:/config/keys/periphery.key"

ENTRYPOINT [ "entrypoint.sh" ]
CMD [ "periphery" ]

# Label to prevent Komodo from stopping with StopAllContainers
LABEL komodo.skip="true"

############################################################
## Komodo CLI (km) on distroless, glibc-matched with the trixie builder
############################################################
FROM gcr.io/distroless/cc-debian13 AS cli

COPY --from=rust-builder /out/km /usr/local/bin/km

ENV KOMODO_CLI_CONFIG_PATHS="/config"

CMD [ "km" ]
