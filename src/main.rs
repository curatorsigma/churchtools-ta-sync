use std::sync::Arc;

use chrono::Utc;
use tokio_util::sync::CancellationToken;

use tracing::{error, info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{filter, fmt::format::FmtSpan};
use tracing_subscriber::{prelude::*, EnvFilter};

mod config;
mod db;
mod get_from_ct;

const BOOKING_DATABASE_NAME: &'static str = ".bookings.db";

/// A single booking for a room
#[derive(Debug, PartialEq)]
struct Booking {
    /// the ID of this booking in CT
    /// This is used for matching ressources against rooms defined in the config.
    churchtools_id: i64,
    /// The booking starts at...
    /// SQLite has no TZ-Aware Datetime type, so this is Naive (timestamp without timezone
    /// information attached).
    /// ALL DATETIMES ARE UTC.
    start_time: chrono::DateTime<Utc>,
    /// The booking ends at...
    end_time: chrono::DateTime<Utc>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(config::Config::create().await?);
    // Setup tracing

    let my_crate_filter = EnvFilter::new("ct_ta_sync");
    let subscriber = tracing_subscriber::registry().with(my_crate_filter).with(
        tracing_subscriber::fmt::layer()
            .compact()
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .with_line_number(true)
            .with_filter(filter::LevelFilter::TRACE),
    );
    tracing::subscriber::set_global_default(subscriber).expect("static tracing config");

    // migrate the database
    sqlx::migrate!().run(&config.db).await?;

    // cancellation token for the two main processes
    let cancel_token = CancellationToken::new();
    // start the data-gatherer
    let gather_handle = tokio::spawn(get_from_ct::keep_db_up_to_date(
        config,
        cancel_token.clone(),
    ));

    // start the data-sender

    match tokio::signal::ctrl_c().await {
        Ok(()) => {
            info!("Received Ctrl-c. Shutting down.");
            cancel_token.cancel();
        }
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
            // we also shut down in case of error
            cancel_token.cancel();
        }
    }

    // Join both tasks
    let (res,) = tokio::join!(gather_handle);
    res?;

    Ok(())
}
