use anyhow::Result;
use aws_sdk_s3::{primitives::ByteStream, Client};
use md5;
use rocket::http::ContentType;
use tokio::io::AsyncReadExt;

#[derive(Debug)]
pub enum S3Error {
    RequestFailed(aws_sdk_s3::Error),
    NoContentLength,
    NoETag,
    ETagMismatch { computed: String, expected: String },
    SizeMismatch { expected: u64, actual: u64 },
    ChunkReadError,
}

pub async fn get_file_from_s3(
    s3_client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(ByteStream, ContentType, u64), S3Error> {
    let resp = s3_client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| {
            eprintln!("S3 GetObject error for key '{}': {:?}", key, e);
            S3Error::RequestFailed(e.into())
        })?;

    let content_type = resp
        .content_type()
        .map(|ct| ct.parse::<ContentType>().unwrap_or(ContentType::Binary))
        .unwrap_or(ContentType::Binary);

    let content_length = resp.content_length().ok_or(S3Error::NoContentLength)?;
    let etag = resp.e_tag().ok_or(S3Error::NoETag)?.to_string();

    let body = resp.body;

    // Check if it's a multipart upload
    if etag.contains("-") {
        // For multipart uploads, we'll check the size
        let mut bytes = Vec::with_capacity(content_length as usize);
        let mut stream = body.into_async_read();

        stream
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| S3Error::ChunkReadError)?;

        let actual_size = bytes.len() as u64;
        if actual_size != content_length as u64 {
            return Err(S3Error::SizeMismatch {
                expected: content_length as u64,
                actual: actual_size,
            });
        }

        Ok((ByteStream::from(bytes), content_type, content_length as u64))
    } else {
        // For single-part uploads, we'll check the ETag
        let mut bytes = Vec::with_capacity(content_length as usize);
        let mut stream = body.into_async_read();

        stream
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| S3Error::ChunkReadError)?;

        let computed_etag = format!("\"{:x}\"", md5::compute(&bytes));

        if computed_etag == etag {
            Ok((ByteStream::from(bytes), content_type, content_length as u64))
        } else {
            Err(S3Error::ETagMismatch {
                computed: computed_etag,
                expected: etag,
            })
        }
    }
}
