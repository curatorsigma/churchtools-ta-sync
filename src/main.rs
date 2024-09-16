mod config;
mod get_from_ct;
mod db;

const BOOKING_DATABASE_NAME: &'static str = ".bookings.db";

/// A single booking for a room
struct Booking {
    /// the ID of this bookin in CT
    churchtools_id: i64,
    /// the name of the room == the name of the ressource in CT
    room: String,
    /// The booking starts at...
    /// SQLite has no TZ-Aware Datetime type, so this is Naive (timestamp without timezone
    /// information attached).
    /// ALL DATETIMES ARE UTC.
    /// Values from Churchtools are coerced to UTC asap.
    start_time: chrono::NaiveDateTime,
    /// The booking ends at...
    end_time: chrono::NaiveDateTime,
}

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
