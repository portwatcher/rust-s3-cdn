use anyhow::Result;
use aws_sdk_s3::primitives::ByteStream;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

pub async fn save_stream_to_disk(file_path: &Path, mut byte_stream: ByteStream) -> Result<()> {
    let mut file = File::create(file_path).await?;

    while let Some(chunk) = byte_stream.next().await {
        let data = chunk?;
        file.write_all(&data).await?;
    }

    Ok(())
}

pub fn generate_file_path(key: &str) -> PathBuf {
    let safe_key = key.replace("/", "_");
    let mut path = PathBuf::from("cached_files");
    path.push(safe_key);
    path
}
