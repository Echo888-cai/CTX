# CTX CLI. Mount the store: docker run --rm -v "$HOME/.ctx:/ctx" -e CTX_HOME=/ctx ctx status
FROM rust:1.80-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p ctx-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/ctx /usr/local/bin/ctx
ENV CTX_HOME=/ctx
VOLUME ["/ctx"]
ENTRYPOINT ["ctx"]
CMD ["status"]
