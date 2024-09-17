use crate::fs::is_key_cached;
use rocket::{head, http::Status};
use std::path::PathBuf;

#[head("/<path..>")]
pub async fn hit(path: PathBuf) -> Result<Status, Status> {
    let key = match path.into_os_string().into_string() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("failed to convert path to string while heading: {:?}", e);
            return Err(Status::BadRequest);
        }
    };
    let s3key = key.replace("\\", "/");
    if is_key_cached(&s3key).await {
        return Ok(Status::Ok);
    }

    Err(Status::NotFound)
}
