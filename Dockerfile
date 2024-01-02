FROM rust:1.74.0

WORKDIR /app
COPY . .

RUN cargo build

ENV TZ="Asia/Tokyo"

CMD ["cargo", "run"]
EXPOSE 8000:8000