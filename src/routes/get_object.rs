use crate::fs::{determine_content_type, generate_file_path, get_cached_file_path};
use crate::s3::get_file_from_s3;
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
    // Check if the path is empty (root path)
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
        if let Ok(file) = fs::File::open(&file_path).await {
            let (file_reader, _) = tokio::io::split(file);
            let file_stream = ReaderStream::new(file_reader);

            let content_type = determine_content_type(&file_path);

            if cfg!(debug_assertions) {
                println!("served {} from cache", &s3key);
            }

            let size = match file_path.metadata() {
                Ok(m) => m.len() as usize,
                Err(e) => {
                    eprintln!("failed to get file size: {}", e);
                    return Err(Status::InternalServerError);
                }
            };

            return Ok(ByteStreamResponse {
                size,
                stream: Box::pin(file_stream),
                content_type,
            });
        }
    }

    println!("{} not cached, fetching from s3", &s3key);

    let bucket = if cfg!(debug_assertions) {
        env::var("TEST_S3_BUCKET_NAME").expect("TEST_S3_BUCKET_NAME must be set")
    } else {
        env::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set")
    };
    match get_file_from_s3(&state.s3_client, &bucket, &s3key).await {
        Ok((mut byte_stream, content_type, content_length, etag)) => {
            let final_file_path = generate_file_path(&s3key);
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
        Err(e) => {
            eprintln!("failed to get file from s3: {}", e);
            Err(Status::NotFound)
        }
    }
}
