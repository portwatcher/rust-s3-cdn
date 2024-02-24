use crate::fs::generate_file_path;
use crate::s3::get_file_from_s3;
use crate::AppState;

use std::{env, path::PathBuf, sync::Arc, pin::Pin};
use tokio::{sync::mpsc, io::AsyncWriteExt};
use tokio_util::io::{StreamReader, ReaderStream};
use rocket::{get, Request, Response, State, http::{ContentType, Status, Header}, response::{self, Responder}};
use tokio_stream::wrappers::ReceiverStream;
use futures::{Stream, StreamExt};
use bytes::Bytes;


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
pub async fn index(
    path: PathBuf,
    state: &State<Arc<AppState>>,
) -> Result<ByteStreamResponse, Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("failed to convert path to string while getting: {:?}", e);
            return Err(Status::BadRequest);
        },
    };

    let s3key = key.replace("\\", "/");
    let app_state = state.inner();

    {
        let mut cache = app_state.cache.lock().await;
        if let Some((file_path, content_type)) = cache.get(&s3key) {
            if let Ok(file) = tokio::fs::File::open(&file_path).await {
                let (file_reader, _) = tokio::io::split(file);
                let file_stream = ReaderStream::new(file_reader);

                if cfg!(debug_assertions) {
                    println!("served {} from cache", &s3key);
                }

                let size = match file_path.metadata() {
                    Ok(m) => m.len() as usize,
                    Err(e) => {
                        eprintln!("failed to get file size: {}", e);
                        return Err(Status::InternalServerError);
                    },
                };

                return Ok(ByteStreamResponse {
                    size,
                    stream: Box::pin(file_stream),
                    content_type: content_type.clone(),
                });
            }
        }
    }

    let bucket = env::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set");
    match get_file_from_s3(&app_state.s3_client, &bucket, &s3key).await {
        Ok((mut byte_stream, content_type, content_length)) => {
            let file_path = generate_file_path(&s3key);

            // Create a new file and an in-memory buffer
            let file = match tokio::fs::File::create(&file_path).await {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("failed to create file: {}", e);
                    return Err(Status::InternalServerError);
                },
            };
            let mut file_writer = tokio::io::BufWriter::new(file);

            // Create a channel
            let (tx, rx) = mpsc::channel(100);
            let file_stream = ReceiverStream::new(rx).map(Ok);

            // Spawn a new task to write to the file
            tokio::spawn(async move {
                while let Some(chunk) = byte_stream.next().await {
                    let chunk = match chunk {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("failed to read from stream: {}", e);
                            break;
                        },
                    };
                    match file_writer.write_all(&chunk).await {
                        Ok(_) => (),
                        Err(e) => {
                            eprintln!("failed to write to file: {}", e);
                            break;
                        },
                    };
                    // Send the chunk to the client
                    if tx.send(chunk).await.is_err() {
                        eprintln!("client disconnected");
                        break;
                    }
                }
                // Close the file writer
                if let Err(e) = file_writer.shutdown().await {
                    eprintln!("failed to close file writer: {}", e);
                }
            });

            // Add to cache
            app_state
                .add_to_cache(s3key.clone(), file_path.clone(), content_type.clone())
                .await
                .expect("add to cache failed");

            if cfg!(debug_assertions) {
                println!("served {} from s3", &s3key);
            }

            Ok(ByteStreamResponse {
                size: content_length,
                stream: Box::pin(file_stream),
                content_type,
            })
        }
        Err(e) => {
            eprintln!("failed to get file from s3: {}", e);
            Err(Status::NotFound)
        },
    }
}