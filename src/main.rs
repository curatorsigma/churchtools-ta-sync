mod config;
mod get_from_ct;

const BOOKING_DATABASE_NAME: &'static str = ".bookings.db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup tracing

    // migrate the database
    let connect_options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(BOOKING_DATABASE_NAME)
        .create_if_missing(true);
    let db = sqlx::SqlitePool::connect_with(connect_options).await?;
    sqlx::migrate!().run(&db).await?;

    // start the data-gatherer

    // start the data-sender

    Ok(())
}
