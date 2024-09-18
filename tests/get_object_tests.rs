use aws_sdk_s3::Client;
use dotenv::dotenv;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client as RocketClient;
use rocket::routes;
use s3_cdn::routes::{get_object, head_object};
use s3_cdn::AppState;
use std::env;

#[tokio::test]
async fn test_get_object() {
    dotenv().ok();

    // Clean up any existing cached file
    let object_key = env::var("TEST_OBJECT_KEY").expect("TEST_OBJECT_KEY must be set");
    let cache_dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let cached_file_path = format!("{}/{}", cache_dir, object_key.replace("/", "_"));
    if std::path::Path::new(&cached_file_path).exists() {
        std::fs::remove_file(&cached_file_path).expect("Failed to remove existing cached file");
    }

    let rocket = setup_rocket().await;
    let client = RocketClient::tracked(rocket)
        .await
        .expect("valid rocket instance");

    // Test GET request
    let response = client.get(format!("/{}", object_key)).dispatch().await;

    assert_eq!(response.status(), Status::Ok);

    // Check if the file was saved locally
    assert!(std::path::Path::new(&cached_file_path).exists());

    // Check content type
    let content_type = response
        .content_type()
        .expect("Content-Type header missing");
    assert_eq!(content_type, ContentType::PNG);

    // Check content length
    let content_length = response
        .headers()
        .get_one("Content-Length")
        .expect("Content-Length header missing");
    assert!(content_length.parse::<usize>().is_ok());

    // Test HEAD request
    let head_response = client.head(format!("/{}", object_key)).dispatch().await;
    assert_eq!(head_response.status(), Status::Ok);
}

#[tokio::test]
async fn test_get_nonexistent_object() {
    dotenv().ok();

    let rocket = setup_rocket().await;
    let client = RocketClient::tracked(rocket)
        .await
        .expect("valid rocket instance");

    let response = client.get("/nonexistent_object").dispatch().await;
    assert_eq!(response.status(), Status::InternalServerError);
}

async fn setup_rocket() -> rocket::Rocket<rocket::Build> {
    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);
    let state = AppState { s3_client };

    rocket::build()
        .manage(state)
        .mount("/", routes![get_object::index, head_object::hit])
}
