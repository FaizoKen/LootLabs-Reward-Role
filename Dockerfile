FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release && rm -rf src
COPY migrations ./migrations
COPY favicon.ico ./favicon.ico
COPY src ./src
RUN touch src/main.rs && cargo build --release && strip target/release/lootlabs-reward-role

FROM alpine:3.21
RUN apk add --no-cache ca-certificates
COPY --from=builder /app/target/release/lootlabs-reward-role /usr/local/bin/lootlabs-reward-role
ENTRYPOINT ["lootlabs-reward-role"]
