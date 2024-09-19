FROM rust:1.80-alpine AS builder
RUN apk add --no-cache build-base
WORKDIR /usr/src/ct-ta-sync
COPY . .
RUN cargo build --release
CMD ["ct-ta-sync"]

FROM alpine:latest
WORKDIR /ct-ta-sync
COPY --from=builder /usr/src/ct-ta-sync/target/release/ct-ta-sync ./
CMD ["./ct-ta-sync"]

