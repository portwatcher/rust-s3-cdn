use aws_sdk_s3::Client;
use dotenv::dotenv;
use rocket::{main, routes};
use tokio::time::Duration;

mod fs;
mod routes;
mod s3;

pub struct AppState {
    s3_client: Client,
}

#[main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();

    let config = aws_config::load_from_env().await;
    let s3_client = Client::new(&config);
    let state = AppState { s3_client };

    let _rocket = rocket::build()
        .manage(state)
        .mount(
            "/",
            routes![routes::get_object::index, routes::head_object::hit],
        )
        .launch()
        .await?;

    tokio::spawn(async move {
        loop {
            // hard coded to 90 days for now
            if let Err(e) = fs::clean_old_files(Duration::from_secs(24 * 60 * 60 * 90)).await {
                eprintln!("Error cleaning old files: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await; // Run every hour
        }
    });

    Ok(())
}
