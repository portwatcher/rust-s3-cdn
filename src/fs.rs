use std::env;
use std::path::{Path, PathBuf};
use infer::Infer;
use rocket::http::ContentType;
use std::io::Read;


pub fn generate_file_path(key: &str) -> PathBuf {
    let safe_key = key.replace("/", "_");
    let dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let mut path = PathBuf::from(dir);
    path.push(safe_key);
    path
}


pub fn generate_key_from_filename(filename: &str) -> String {
    filename.replace("_", "/")
}


pub fn is_key_cached(key: &str) -> bool {
    let path = generate_file_path(key);
    println!("checking if file exists: {:?}", path);
    path.exists()
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
            _ => ContentType::Binary, // Default content type
        }
    } else {
        ContentType::Binary // Default if unable to infer
    }
}