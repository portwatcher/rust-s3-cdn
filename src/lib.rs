pub mod fs;
pub mod routes;
pub mod s3;

pub struct AppState {
    pub s3_client: aws_sdk_s3::Client,
}
