use anyhow::Result;
use aws_sdk_s3::{primitives::ByteStream, Client};
use md5;
use rocket::http::ContentType;

#[derive(Debug)]
pub enum S3Error {
    RequestFailed(String),
    NoContentLength,
    NoETag,
    ETagMismatch,
    ChunkReadError(String),
}

pub async fn get_file_from_s3(
    s3_client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(ByteStream, ContentType, usize, String), S3Error> {
    match s3_client.get_object().bucket(bucket).key(key).send().await {
        Ok(resp) => {
            let content_type = resp
                .content_type()
                .map(|ct| ct.parse::<ContentType>().unwrap_or(ContentType::Binary))
                .unwrap_or(ContentType::Binary);

            let content_length = resp.content_length().ok_or(S3Error::NoContentLength)? as usize;
            let etag = resp.e_tag().ok_or(S3Error::NoETag)?.to_string();

            let mut body = resp.body;
            let mut bytes = Vec::with_capacity(content_length);

            while let Some(chunk) = body
                .try_next()
                .await
                .map_err(|e| S3Error::ChunkReadError(e.to_string()))?
            {
                bytes.extend_from_slice(&chunk);
            }

            let computed_etag = format!("\"{:x}\"", md5::compute(&bytes));

            if computed_etag == etag {
                Ok((ByteStream::from(bytes), content_type, content_length, etag))
            } else {
                Err(S3Error::ETagMismatch)
            }
        }
        Err(e) => Err(S3Error::RequestFailed(e.to_string())),
    }
}

pub async fn get_object_etag(
    s3_client: &Client,
    bucket: &str,
    key: &str,
) -> Result<String, S3Error> {
    match s3_client.head_object().bucket(bucket).key(key).send().await {
        Ok(resp) => resp
            .e_tag()
            .ok_or(S3Error::NoETag)
            .map(|etag| etag.to_string()),
        Err(e) => Err(S3Error::RequestFailed(e.to_string())),
    }
}
