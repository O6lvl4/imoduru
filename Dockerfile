FROM rust:1.82-bookworm AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release

FROM node:22-bookworm-slim

RUN npx playwright install --with-deps chromium

WORKDIR /app
COPY --from=builder /build/target/release/imoduru /usr/local/bin/imoduru
COPY bridge/ bridge/
RUN cd bridge && npm install --production

ENTRYPOINT ["imoduru"]
