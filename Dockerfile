FROM rust:1.82-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY binary ./binary
COPY src ./src

RUN cargo build --release -p agent-team

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    curl \
    git \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g @anthropic-ai/claude-code

COPY --from=builder /app/target/release/agent-team /usr/local/bin/

WORKDIR /workspace
CMD ["agent-team"]
