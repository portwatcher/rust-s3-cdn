mod fs;
mod s3;

use crate::fs::{generate_file_path, save_stream_to_disk};
use crate::s3::get_file_from_s3;

use aws_sdk_s3::Client;
use dotenv::dotenv;
use lru::LruCache;
use rocket::{get, http::ContentType, http::Status, main, routes, State};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

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
}

#[get("/<path..>")]
async fn index(
    path: PathBuf,
    state: &State<Arc<AppState>>,
) -> Result<(ContentType, Vec<u8>), Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(_) => return Err(Status::BadRequest),
    };

    let s3key = key.replace("\\", "/");
    let app_state = state.inner();

    {
        let mut cache = app_state.cache.lock().await;
        if let Some((file_path, content_type)) = cache.get(&s3key) {
            if let Ok(data) = tokio::fs::read(&file_path).await {
                return Ok((content_type.clone(), data));
            }
        }
    }

    let bucket = dotenv::var("S3_BUCKET_NAME").expect("S3_BUCKET_NAME must be set");
    match get_file_from_s3(&app_state.s3_client, &bucket, &s3key).await {
        Ok((byte_stream, content_type)) => {
            let file_path = generate_file_path(&s3key);

            // Save the stream to disk
            if let Err(e) = save_stream_to_disk(&file_path, byte_stream).await {
                eprintln!("Failed to save file: {}", e);
                return Err(Status::InternalServerError);
            }

            // Add to cache and read the file to send to the client
            app_state
                .add_to_cache(s3key.clone(), file_path.clone(), content_type.clone())
                .await
                .expect("add to cache failed");
            let data = std::fs::read(&file_path).expect("Failed to read file");
            Ok((content_type, data))
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

    let _rocket = rocket::build()
        .manage(state)
        .mount("/", routes![index])
        .launch()
        .await?;

    Ok(())
}
