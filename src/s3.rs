use anyhow::{Context, Result};
use aws_sdk_s3::{primitives::ByteStream, Client};
use rocket::http::ContentType;

pub async fn get_file_from_s3(
    s3_client: &Client,
    bucket: &str,
    key: &str,
) -> Result<(ByteStream, ContentType)> {
    let resp = s3_client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .context("Failed to send request to S3")?;

    let content_type = resp
        .content_type()
        .map(|ct| ct.parse::<ContentType>().unwrap_or(ContentType::Binary))
        .unwrap_or(ContentType::Binary);

    Ok((resp.body, content_type))
}
