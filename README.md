# rust-s3-cdn

A simple LRU cached proxy for AWS S3 written in Rust

## why

This project can save your money by two means:

- This project can reduce the requests and traffic to s3 comparing to returning signed url directly to the clients.
- This project can increase fetch speed when you deploy it near to users while getting rid of cloudfront

## caveats

Please note that this project does not process authentication, it is intended to be used behind a gateway where authentication is done.

Authentication is specific to your business and beyond the scope of this project.

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
