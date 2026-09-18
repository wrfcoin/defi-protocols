FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p defi-node

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /src/target/release/defi-node /usr/local/bin/defi-node
COPY --from=builder /src/crates/defi-node/fixtures/pools.toml /etc/defi-node/pools.toml
ENV DEFI_NODE_POOLS=/etc/defi-node/pools.toml
EXPOSE 7100
ENTRYPOINT ["/usr/local/bin/defi-node"]
