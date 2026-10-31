# The Bullseye sync server. Built from the repository's root, as it uses the app's own modules:
#
#   docker build -f server/Dockerfile -t bullseye .
#
# Plain HTTP on port 8080: put it behind your own reverse proxy for HTTPS. See server/README.md.

FROM rust:1-bookworm AS build
WORKDIR /src
# Only what the server is built from (`server/Cargo.toml` says why these).
COPY src-tauri/src src-tauri/src
COPY src/strings.js src/strings.js
COPY scripts/linux-tests/src scripts/linux-tests/src
COPY scripts/linux-tests/windows-sys scripts/linux-tests/windows-sys
COPY server server
# Cargo.lock pins the dependencies (not --locked: a release writes its version into Cargo.toml).
RUN cargo build --release --manifest-path server/Cargo.toml

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --no-create-home bullseye && mkdir /data && chown bullseye /data
COPY --from=build /src/server/target/release/bullseye-sync /usr/local/bin/bullseye-sync
# The window's page, served at / for managing every computer from a browser (`BULLSEYE_PAGE`).
COPY src /app/web
USER bullseye
ENV BULLSEYE_DATA=/data BULLSEYE_LISTEN=0.0.0.0:8080 BULLSEYE_PAGE=/app/web
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["bullseye-sync"]
