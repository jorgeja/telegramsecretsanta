FROM rust:1-slim-bookworm AS builder
WORKDIR /app

# Build dependencies on their own layer so code changes don't recompile them.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src target/release/deps/secret_santa*

COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --no-create-home santa \
    && mkdir /data && chown santa /data
COPY --from=builder /app/target/release/secret_santa /usr/local/bin/secret_santa

USER santa
ENV SANTA_STATE_FILE=/data/santa_state.json \
    RUST_LOG=info
VOLUME /data

# The bot shuts down cleanly on Ctrl+C, so have `docker stop` send that instead of SIGTERM.
STOPSIGNAL SIGINT
CMD ["secret_santa"]
