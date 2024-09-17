use infer::Infer;
use rocket::http::ContentType;
use std::env;
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::fs;

pub async fn is_key_cached(key: &str) -> bool {
    let cache_dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let dir = std::path::Path::new(&cache_dir);
    let safe_key = key.replace("/", "_");

    if let Ok(mut entries) = fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_str().unwrap();
            if file_name_str.starts_with(&safe_key)
                && (file_name_str == safe_key
                    || file_name_str.starts_with(&format!("{}_", safe_key)))
            {
                return true;
            }
        }
    }
    false
}

pub async fn get_cached_file_path(key: &str) -> Option<PathBuf> {
    let cache_dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let dir = std::path::Path::new(&cache_dir);
    let safe_key = key.replace("/", "_");

    if let Ok(mut entries) = fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_name = entry.file_name();
            let file_name_str = file_name.to_str().unwrap();
            if file_name_str.starts_with(&safe_key)
                && (file_name_str == safe_key
                    || file_name_str.starts_with(&format!("{}_", safe_key)))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

pub fn determine_content_type(path: &Path) -> ContentType {
    let mut buf = [0; 10]; // buffer to read file's initial bytes
    if let Ok(mut file) = std::fs::File::open(path) {
        let _ = file.read(&mut buf);
    }

    let infer = Infer::new();
    if let Some(kind) = infer.get(&buf) {
        match kind.mime_type() {
            "image/jpeg" => ContentType::JPEG,
            "image/png" => ContentType::PNG,
            "image/webp" => ContentType::WEBP,
            "image/tiff" => ContentType::TIFF,
            "video/mp4" => ContentType::MP4,
            "video/mpeg" => ContentType::MPEG,
            "image/gif" => ContentType::GIF,
            "application/zip" => ContentType::ZIP,
            "application/pdf" => ContentType::PDF,
            "application/epub+zip" => ContentType::EPUB,
            "audio/mpeg" => ContentType::MP3,
            "application/gzip" => ContentType::GZIP,
            "audio/ogg" => ContentType::OGG,
            "application/vnd.rar" => ContentType::RAR,
            "text/plain" => ContentType::Plain,
            "text/html" => ContentType::HTML,
            "text/css" => ContentType::CSS,
            "application/javascript" => ContentType::JavaScript,
            "application/json" => ContentType::JSON,
            "application/xml" => ContentType::XML,
            "application/octet-stream" => ContentType::Binary,
            "image/svg+xml" => ContentType::SVG,
            "video/webm" => ContentType::WEBM,
            _ => ContentType::Binary, // Default content type
        }
    } else {
        ContentType::Binary // Default if unable to infer
    }
}

pub async fn clean_old_files(max_age: std::time::Duration) -> Result<(), std::io::Error> {
    let dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let cache_dir = Path::new(&dir);
    let mut entries = fs::read_dir(cache_dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_file() {
            if let Ok(metadata) = fs::metadata(&path).await {
                if let Ok(modified) = metadata.modified() {
                    if modified.elapsed().unwrap_or(std::time::Duration::ZERO) > max_age {
                        fs::remove_file(&path).await?;
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn generate_file_path(key: &str, etag: &str) -> PathBuf {
    let safe_key = key.replace("/", "_");
    let dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let mut path = PathBuf::from(dir);
    path.push(format!("{}_{}", safe_key, etag.trim_matches('"')));
    path
}

pub fn extract_etag_from_filename(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once('_'))
        .map(|(_, etag)| format!("\"{}\"", etag))
}
