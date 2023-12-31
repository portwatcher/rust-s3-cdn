# rust-s3-cdn

A simple LRU cached proxy for AWS S3 written in Rust

## .env configuration

Before `cargo run`, you need to create a `.env` file under root dir of this project that goes:

```env
AWS_ACCESS_KEY_ID=foo
AWS_SECRET_ACCESS_KEY=bar
AWS_DEFAULT_REGION=baz
S3_BUCKET_NAME=qux
```

## Todo

dockerize
