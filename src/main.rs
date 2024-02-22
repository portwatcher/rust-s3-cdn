use crate::fs::{
    generate_file_path,
    determine_content_type,
    generate_key_from_filename,
    is_key_cached,
};
use crate::s3::get_file_from_s3;

use std::{env, path::{PathBuf, Path}, sync::Arc};
use tokio::{sync::Mutex, io::{AsyncWriteExt, ReadHalf}, fs::File};
use tokio_util::io::{StreamReader, ReaderStream};
use rocket::{get, head, main, routes, Request, Response, State, http::{ContentType, Status, Header}, response::{self, Responder}};
use aws_sdk_s3::Client;
use dotenv::dotenv;
use lru::LruCache;

mod fs;
mod s3;

struct AppState {
    cache: Mutex<LruCache<String, (PathBuf, ContentType)>>,
    s3_client: Client,
}

impl AppState {
    async fn add_to_cache(
        &self,
        key: String,
        file_path: PathBuf,
        content_type: ContentType,
    ) -> Result<(), anyhow::Error> {
        let mut cache = self.cache.lock().await;

        if cache.len() == cache.cap().get() {
            if let Some((_evicted_key, (evicted_file_path, _))) = cache.pop_lru() {
                tokio::fs::remove_file(&evicted_file_path).await?;
            }
        }

        cache.put(key, (file_path, content_type));
        Ok(())
    }

    async fn rebuild_cache_from_disk(&self) -> Result<(), anyhow::Error> {
        let dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
        let cache_dir = Path::new(&dir);
        let mut cache = self.cache.lock().await;
        let mut paths = tokio::fs::read_dir(cache_dir).await?;

        let mut count = 0;
        while let Some(entry) = paths.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                let filename = path.file_name().unwrap().to_str().unwrap().to_owned();
                let content_type = determine_content_type(&path);
                let key = generate_key_from_filename(&filename);
                cache.put(key, (path, content_type));
                count += 1;
            }
        }

        println!("rebuild lru cache done, cached {} files", count);

        Ok(())
    }
}


#[head("/<path..>")]
async fn hit(path: PathBuf) -> Result<Status, Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(_) => return Err(Status::BadRequest),
    };
    let s3key = key.replace("\\", "/");
    if is_key_cached(&s3key) {
        return Ok(Status::Ok);
    }

    Err(Status::NotFound)
}

struct ByteStreamResponse {
    size: usize,
    stream: ReaderStream<ReadHalf<File>>,
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
async fn index(
    path: PathBuf,
    state: &State<Arc<AppState>>,
) -> Result<ByteStreamResponse, Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(_) => return Err(Status::BadRequest),
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
                    Err(_) => 0,
                };

                return Ok(ByteStreamResponse {
                    size,
                    stream: file_stream,
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
                Err(_) => return Err(Status::InternalServerError),
            };
            let (file_reader, mut file_writer) = tokio::io::split(file);

            // Duplicate the stream
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(_) => return Err(Status::InternalServerError),
                };
                match file_writer.write_all(&chunk).await {
                    Ok(_) => (),
                    Err(_) => return Err(Status::InternalServerError),
                };
            }

            // Add to cache
            app_state
                .add_to_cache(s3key.clone(), file_path.clone(), content_type.clone())
                .await
                .expect("add to cache failed");

            if cfg!(debug_assertions) {
                println!("served {} from s3", &s3key);
            }
            let file_stream = ReaderStream::new(file_reader);

            Ok(ByteStreamResponse {
                size: content_length,
                stream: file_stream,
                content_type,
            })
        }
        Err(_) => Err(Status::NotFound),
    }
}

#[main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();

    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);

    let cache_capacity_string = dotenv::var("CACHE_CAPACITY").unwrap_or(1000.to_string());
    let cache_capacity = cache_capacity_string
        .parse::<usize>()
        .expect("Invalid CACHE_CAPACITY value")
        .try_into()
        .expect("Cache capacity must be non-zero");

    let state = Arc::new(AppState {
        cache: Mutex::new(LruCache::new(cache_capacity)),
        s3_client,
    });

    state.rebuild_cache_from_disk().await.expect("rebuild cache failed");

    let _rocket = rocket::build()
        .manage(state)
        .mount("/", routes![index])
        .mount("/", routes![hit])
        .launch()
        .await?;

    Ok(())
}
