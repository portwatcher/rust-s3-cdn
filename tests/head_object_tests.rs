use aws_sdk_s3::Client;
use dotenv::dotenv;
use rocket::http::Status;
use rocket::local::asynchronous::Client as RocketClient;
use rocket::routes;
use s3_cdn::routes::{get_object, head_object};
use s3_cdn::AppState;
use std::env;

#[tokio::test]
async fn test_head_existing_object() {
    dotenv().ok();

    let rocket = setup_rocket().await;
    let client = RocketClient::tracked(rocket)
        .await
        .expect("valid rocket instance");

    let object_key = env::var("TEST_OBJECT_KEY").expect("TEST_OBJECT_KEY must be set");
    let response = client.head(format!("/{}", object_key)).dispatch().await;
    assert_eq!(response.status(), Status::Ok);
}

#[tokio::test]
async fn test_head_nonexistent_object() {
    dotenv().ok();

    let rocket = setup_rocket().await;
    let client = RocketClient::tracked(rocket)
        .await
        .expect("valid rocket instance");

    let response = client.head("/nonexistent_object").dispatch().await;
    assert_eq!(response.status(), Status::NotFound);
}

async fn setup_rocket() -> rocket::Rocket<rocket::Build> {
    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);
    let state = AppState { s3_client };

    rocket::build()
        .manage(state)
        .mount("/", routes![get_object::index, head_object::hit])
}
