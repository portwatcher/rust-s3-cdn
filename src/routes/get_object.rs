use crate::fs::{determine_content_type, generate_file_path, get_cached_file_path};
use crate::s3::{get_file_from_s3, S3Error};
use crate::AppState;

use aws_sdk_s3::{error::ProvideErrorMetadata, primitives::ByteStream, Client};
use bytes::Bytes;
use futures::Stream;
use rocket::{
    get,
    http::{ContentType, Header, Status},
    response::{self, Responder},
    Request, Response, State,
};
use std::{env, path::PathBuf, pin::Pin};
use tokio::{fs, io::AsyncWriteExt};
use tokio_util::io::{ReaderStream, StreamReader};
use uuid::Uuid;

pub struct ByteStreamResponse {
    size: usize,
    stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static>>,
    content_type: ContentType,
}

#[rocket::async_trait]
impl<'r> Responder<'r, 'static> for ByteStreamResponse {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let reader = StreamReader::new(self.stream);

        Response::build()
            .header(self.content_type)
            .header(Header::new("Content-Length", self.size.to_string()))
            .streamed_body(reader)
            .ok()
    }
}

async fn get_file_with_retry(
    s3_client: &Client,
    bucket: &str,
    s3key: &str,
    max_retries: usize,
) -> Result<(ByteStream, ContentType, u64), S3Error> {
    let mut retries = 0;
    loop {
        match get_file_from_s3(s3_client, bucket, s3key).await {
            Ok(result) => return Ok(result),
            Err(S3Error::ETagMismatch { computed, expected }) => {
                eprintln!(
                    "ETag mismatch for key '{}'. Computed: {}, Expected: {}. Retry {} of {}",
                    s3key,
                    computed,
                    expected,
                    retries + 1,
                    max_retries
                );
                if retries >= max_retries {
                    return Err(S3Error::ETagMismatch { computed, expected });
                }
                retries += 1;
            }
            Err(S3Error::SizeMismatch { expected, actual }) => {
                eprintln!(
                    "Size mismatch for key '{}'. Expected: {}, Actual: {}. Retry {} of {}",
                    s3key,
                    expected,
                    actual,
                    retries + 1,
                    max_retries
                );
                if retries >= max_retries {
                    return Err(S3Error::SizeMismatch { expected, actual });
                }
                retries += 1;
            }
            Err(S3Error::ChunkReadError) => {
                eprintln!("Network error: Failed to read file data from S3");
                if retries >= max_retries {
                    return Err(S3Error::ChunkReadError);
                }
                retries += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[get("/<path..>")]
pub async fn index(path: PathBuf, state: &State<AppState>) -> Result<ByteStreamResponse, Status> {
    if path.as_os_str().is_empty() {
        return Err(Status::NotFound);
    }

    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("failed to convert path to string while getting: {:?}", e);
            return Err(Status::BadRequest);
        }
    };

    // Early return for favicon.ico requests
    if key == "favicon.ico" {
        return Err(Status::NotFound);
    }

    let s3key = key.replace("\\", "/");

    if let Some(file_path) = get_cached_file_path(&s3key).await {
        if let Ok(file) = fs::File::open(&file_path).await {
            let content_type = determine_content_type(&file_path);
            let metadata = file.metadata().await.map_err(|e| {
                eprintln!("failed to get file metadata: {}", e);
                Status::InternalServerError
            })?;
            let size = metadata.len() as usize;

            let (file_reader, _) = tokio::io::split(file);
            let file_stream = ReaderStream::new(file_reader);

            if cfg!(debug_assertions) {
                println!("served {} from cache", &s3key);
            }

            return Ok(ByteStreamResponse {
                size,
                stream: Box::pin(file_stream),
                content_type,
            });
        }
    }

    let bucket = if cfg!(debug_assertions) {
        env::var("TEST_S3_BUCKET_NAME").expect("TEST_S3_BUCKET_NAME must be set")
    } else {
        env::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set")
    };

    match get_file_with_retry(&state.s3_client, &bucket, &s3key, 5).await {
        Ok((mut byte_stream, content_type, content_length)) => {
            let file_path = generate_file_path(&s3key);
            let tmp_file_path = file_path.with_file_name(format!("{}.tmp", Uuid::new_v4()));

            // Create a new temporary file
            let file = match fs::File::create(&tmp_file_path).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("failed to create temporary file: {}", e);
                    return Err(Status::InternalServerError);
                }
            };
            let mut file_writer = tokio::io::BufWriter::new(file);

            // Write the file content
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("failed to read from stream: {}", e);
                        return Err(Status::InternalServerError);
                    }
                };
                if let Err(e) = file_writer.write_all(&chunk).await {
                    eprintln!("failed to write to file: {}", e);
                    return Err(Status::InternalServerError);
                }
            }

            // Close the file writer
            if let Err(e) = file_writer.shutdown().await {
                eprintln!("failed to close file writer: {}", e);
                return Err(Status::InternalServerError);
            }

            // Rename the temporary file to the final file name
            if let Err(e) = fs::rename(&tmp_file_path, &file_path).await {
                eprintln!("failed to rename temporary file: {}", e);
                return Err(Status::InternalServerError);
            }

            if cfg!(debug_assertions) {
                println!("served {} from s3", &s3key);
            }

            Ok(ByteStreamResponse {
                size: content_length as usize,
                stream: Box::pin(tokio_util::io::ReaderStream::new(
                    tokio::fs::File::open(&file_path).await.unwrap(),
                )),
                content_type,
            })
        }
        Err(e) => {
            eprintln!(
                "Failed to get file from S3 for key '{}'. Detailed error: {:?}",
                s3key, e
            );
            match e {
                S3Error::RequestFailed(err) => {
                    let error_code = err.code().unwrap_or("Unknown");
                    let error_message = err.message().unwrap_or("No error message");
                    eprintln!("S3 request failed: {} - {}", error_code, error_message);

                    match error_code {
                        "NoSuchKey" => return Err(Status::NotFound),
                        "NoSuchBucket" => {
                            eprintln!("Configuration error: S3 bucket does not exist");
                            return Err(Status::InternalServerError);
                        }
                        "AccessDenied" => {
                            eprintln!("Configuration error: No permission to access S3 bucket");
                            return Err(Status::InternalServerError);
                        }
                        _ => {
                            eprintln!("S3 request failed with error: {:?}", err);
                            return Err(Status::InternalServerError);
                        }
                    }
                }
                S3Error::ETagMismatch { computed, expected } => {
                    eprintln!("File integrity error - ETag mismatch");
                    eprintln!("Expected: {}, Got: {}", expected, computed);
                }
                S3Error::SizeMismatch { expected, actual } => {
                    eprintln!("File integrity error - Size mismatch");
                    eprintln!("Expected: {} bytes, Got: {} bytes", expected, actual);
                }
                S3Error::NoContentLength => {
                    eprintln!("S3 response error: Missing Content-Length header");
                }
                S3Error::NoETag => {
                    eprintln!("S3 response error: Missing ETag header");
                }
                S3Error::ChunkReadError => {
                    eprintln!("Network error: Failed to read file data from S3");
                }
            }
            Err(Status::InternalServerError)
        }
    }
}
