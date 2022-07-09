FROM rust:1.62 as builder

WORKDIR /app

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

COPY ./src ./src

RUN cargo build --release

FROM gcr.io/distroless/cc

COPY --from=builder /app/target/release/icalul8r .

CMD ["./icalul8r"]
