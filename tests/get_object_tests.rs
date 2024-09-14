use aws_sdk_s3::Client;
use dotenv::dotenv;
use rocket::http::Status;
use rocket::local::asynchronous::Client as RocketClient;
use rocket::routes;
use s3_cdn::routes::get_object;
use s3_cdn::AppState;
use std::env;

#[tokio::test]
async fn test_get_object() {
    dotenv().ok();

    // Load AWS configuration from environment
    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);

    let state = AppState { s3_client };

    let rocket = rocket::build()
        .manage(state)
        .mount("/", routes![get_object::index]);

    let client = RocketClient::tracked(rocket)
        .await
        .expect("valid rocket instance");

    let object_key = env::var("TEST_OBJECT_KEY").expect("TEST_OBJECT_KEY must be set");
    println!("object_key: {}", object_key);
    let response = client.get(format!("/{}", object_key)).dispatch().await;

    assert_eq!(response.status(), Status::Ok);

    // Check if the file was saved locally
    let cache_dir = env::var("CACHE_DIR").expect("CACHE_DIR must be set");
    let cached_file_path = format!("{}/{}", cache_dir, object_key.replace("/", "_"));
    println!("cached_file_path: {}", cached_file_path);
    assert!(std::path::Path::new(&cached_file_path).exists());
}
