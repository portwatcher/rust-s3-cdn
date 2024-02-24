use crate::fs::{
    determine_content_type,
    generate_key_from_filename,
};

use std::{env, path::{PathBuf, Path}, sync::Arc};
use tokio::sync::Mutex;
use rocket::{main, routes, http::ContentType};
use aws_sdk_s3::Client;
use dotenv::dotenv;
use lru::LruCache;


mod fs;
mod s3;
mod routes;

pub struct AppState {
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
        .mount("/", routes![routes::get_object::index, routes::head_object::hit])
        .launch()
        .await?;

    Ok(())
}
