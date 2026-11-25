# tiffin-helper as one small image. Build it from the repository root of a release snapshot
# (it compiles app/ too, and only the snapshot carries the redistributable MET stand-in):
#   docker build -f helper/Dockerfile -t tiffin-helper .
FROM rust:1-slim-bookworm AS build
# native-tls uses the system OpenSSL on Linux
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY app app
COPY helper helper
RUN cargo build --release --locked --manifest-path helper/Cargo.toml

FROM debian:bookworm-slim
# No curl on purpose: the importer's curl fallback then simply doesn't run.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libssl3 && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home --shell /usr/sbin/nologin tiffin
COPY --from=build /src/helper/target/release/tiffin-helper /usr/local/bin/tiffin-helper
USER tiffin
ENV TIFFIN_HELPER_ADDR=0.0.0.0:8787
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/tiffin-helper"]
