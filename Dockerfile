# Stage 1: Build binary
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

# Stage 2: Minimal runtime image (<20MB)
FROM alpine:3.20
RUN apk add --no-cache ca-certificates tzdata
WORKDIR /app
COPY --from=builder /app/target/release/ferry /usr/local/bin/ferry
COPY ferry.toml /app/ferry.toml

EXPOSE 4000
ENV FERRY_CONFIG=/app/ferry.toml
CMD ["ferry"]
