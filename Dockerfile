FROM rust:1.74.0 as builder

WORKDIR /app
COPY . .

RUN cargo build --release


FROM rust:1.74.0

WORKDIR /app
COPY --from=builder /app/target/release/s3-cdn .
COPY --from=builder /app/.env .

ENV TZ="Asia/Tokyo"

CMD ["./s3-cdn"]
EXPOSE 8000:8000