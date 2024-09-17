use crate::fs::{
    determine_content_type, extract_etag_from_filename, generate_file_path, get_cached_file_path,
};
use crate::s3::{get_file_from_s3, S3Error};
use crate::AppState;

use bytes::Bytes;
use futures::Stream;
use md5;
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

    let s3key = key.replace("\\", "/");

    // Check if the file is already cached
    if let Some(file_path) = get_cached_file_path(&s3key).await {
        let etag = extract_etag_from_filename(&file_path);
        if let Ok(file) = fs::File::open(&file_path).await {
            let content = fs::read(&file_path).await.map_err(|e| {
                eprintln!("failed to read cached file: {}", e);
                Status::InternalServerError
            })?;

            if let Some(etag) = etag {
                let calculated_md5 = format!("\"{:x}\"", md5::compute(&content));
                if calculated_md5 != etag {
                    // ETag mismatch, delete the cached file
                    if let Err(e) = fs::remove_file(&file_path).await {
                        eprintln!("failed to remove outdated cached file: {}", e);
                    }
                    // Continue to fetch from S3
                } else {
                    // ETag matches, serve the file
                    let (file_reader, _) = tokio::io::split(file);
                    let file_stream = ReaderStream::new(file_reader);
                    let content_type = determine_content_type(&file_path);
                    let size = content.len();

                    if cfg!(debug_assertions) {
                        println!("served {} from cache", &s3key);
                    }

                    return Ok(ByteStreamResponse {
                        size,
                        stream: Box::pin(file_stream),
                        content_type,
                    });
                }
            } else {
                // No ETag in filename, serve the file without checking
                let (file_reader, _) = tokio::io::split(file);
                let file_stream = ReaderStream::new(file_reader);
                let content_type = determine_content_type(&file_path);
                let size = content.len();

                if cfg!(debug_assertions) {
                    println!("served {} from cache (legacy format)", &s3key);
                }

                return Ok(ByteStreamResponse {
                    size,
                    stream: Box::pin(file_stream),
                    content_type,
                });
            }
        }
    }

    println!("{} not cached or outdated, fetching from s3", &s3key);

    let bucket = if cfg!(debug_assertions) {
        env::var("TEST_S3_BUCKET_NAME").expect("TEST_S3_BUCKET_NAME must be set")
    } else {
        env::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set")
    };
    match get_file_from_s3(&state.s3_client, &bucket, &s3key).await {
        Ok((mut byte_stream, content_type, content_length, etag)) => {
            let final_file_path = generate_file_path(&s3key, &etag);
            let tmp_file_path = final_file_path.with_file_name(format!("{}.tmp", Uuid::new_v4()));

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

            // Verify checksum
            let file_content = fs::read(&tmp_file_path).await.map_err(|e| {
                eprintln!("failed to read temporary file: {}", e);
                Status::InternalServerError
            })?;
            let file_md5 = format!("\"{:x}\"", md5::compute(&file_content));

            if file_md5 == etag {
                // Rename the temporary file to the final file name
                if let Err(e) = fs::rename(&tmp_file_path, &final_file_path).await {
                    eprintln!("failed to rename temporary file: {}", e);
                    return Err(Status::InternalServerError);
                }
            } else {
                eprintln!("Checksum mismatch. Expected: {}, Got: {}", etag, file_md5);
                if let Err(e) = fs::remove_file(&tmp_file_path).await {
                    eprintln!("failed to remove temporary file: {}", e);
                }
                return Err(Status::InternalServerError);
            }

            if cfg!(debug_assertions) {
                println!("served {} from s3", &s3key);
            }

            Ok(ByteStreamResponse {
                size: content_length,
                stream: Box::pin(tokio_util::io::ReaderStream::new(
                    tokio::fs::File::open(&final_file_path).await.unwrap(),
                )),
                content_type,
            })
        }
        Err(e) => match e {
            S3Error::RequestFailed(err_msg) => {
                eprintln!("S3 request failed: {}", err_msg);
                Err(Status::InternalServerError)
            }
            S3Error::NoContentLength => {
                eprintln!("No content length in S3 response");
                Err(Status::InternalServerError)
            }
            S3Error::NoETag => {
                eprintln!("No ETag in S3 response");
                Err(Status::InternalServerError)
            }
            S3Error::ETagMismatch => {
                eprintln!("ETag mismatch when fetching from S3");
                Err(Status::InternalServerError)
            }
            S3Error::ChunkReadError(err_msg) => {
                eprintln!("Error reading chunk from S3: {}", err_msg);
                Err(Status::InternalServerError)
            }
        },
    }
}
