FROM rust:1.84.1 AS builder

WORKDIR /app
COPY . .

# Set RUSTFLAGS to ensure compatible CPU target
ENV RUSTFLAGS="-C target-cpu=generic"
RUN cargo build --release


FROM rust:1.84.1

WORKDIR /app
COPY --from=builder /app/target/release/s3-cdn .

ENV TZ="Asia/Tokyo"

CMD ["./s3-cdn"]
EXPOSE 8000:8000