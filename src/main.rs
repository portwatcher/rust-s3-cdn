mod s3;
mod fs;

use crate::s3::get_file_from_s3;
use crate::fs::{save_stream_to_disk, generate_file_path};

use rocket::{get, routes, main, State, http::Status, http::ContentType};
use std::path::PathBuf;
use dotenv::dotenv;
use aws_sdk_s3::Client;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Mutex, Arc};
use std::fs::remove_file;

struct AppState {
    cache: Mutex<LruCache<String, (PathBuf, ContentType)>>,
    s3_client: Client,
}

impl AppState {
    fn add_to_cache(&self, key: String, file_path: PathBuf, content_type: ContentType) {
        let mut cache = self.cache.lock().unwrap();

        if cache.len() == cache.cap().get() {
            if let Some((_evicted_key, (evicted_file_path, _))) = cache.pop_lru() {
                let _ = remove_file(&evicted_file_path);
            }
        }

        cache.put(key, (file_path, content_type));
    }
}


#[get("/<path..>")]
async fn index(path: PathBuf, state: &State<Arc<AppState>>) -> Result<(ContentType, Vec<u8>), Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(_) => return Err(Status::BadRequest),
    };

    let s3key = key.replace("\\", "/");
    let app_state = state.inner();
    
    {
        let mut cache = match app_state.cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                eprintln!("Cache lock is poisoned. Using recovered state.");
                poisoned.into_inner()
            }
        };
        if let Some((file_path, content_type)) = cache.get(&s3key) {
            if let Ok(data) = std::fs::read(&file_path) {
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
            app_state.add_to_cache(s3key.clone(), file_path.clone(), content_type.clone());
            let data = std::fs::read(&file_path).expect("Failed to read file");
            Ok((content_type, data))
        },
        Err(_) => Err(Status::NotFound),
    }
}


#[main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();

    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);

    let cache_capacity = NonZeroUsize::new(1000).expect("Cache capacity must be non-zero");

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
