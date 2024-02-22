# rust-s3-cdn

A simple LRU cached proxy for AWS S3 written in Rust

## why

This project can save your money by two means:

- This project can reduce the requests and traffic to s3 comparing to returning signed url directly to the clients.
- This project can increase fetch speed when you deploy it near to users to get rid of cloudfront fees

## caveats

Please note that this project does not process authentication, it is intended to be used behind a gateway where authentication is done first.

Authentication is specific to your business and beyond the scope of this project.

## environment configuration

Before `cargo run`, you need to create a `.env` file under root dir of this project that goes:

```env
CACHE_DIR=cached_files
AWS_ACCESS_KEY_ID=foo
AWS_SECRET_ACCESS_KEY=bar
AWS_DEFAULT_REGION=baz
S3_BUCKET_NAME=qux
CACHE_CAPACITY=1000
```

or set these values in other config files like `docker-compose.yml`.
They are essential for this program to run.


## API

```
GET /:path
```

Return a cached file. If it is not cached, pull from s3.


```
HEAD /:path
```

Check if a file is cached


## Todo

- ~~dockerize~~
- invalidate interface
- ~~restore lru object from files already cached~~
- a background worker that auto removes files according to configured timeout
- ~~use stream~~
