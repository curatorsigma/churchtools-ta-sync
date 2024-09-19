use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use tracing::{error, info};
use tracing_subscriber::{filter, fmt::format::FmtSpan};
use tracing_subscriber::{prelude::*, EnvFilter};

mod config;
mod db;
mod pull_from_ct;
mod push_to_ta;
mod read_ext_temp;

const BOOKING_DATABASE_NAME: &'static str = ".bookings.db";

/// A single booking for a room
#[derive(Debug, PartialEq)]
struct Booking {
    /// the ID of this booking in CT
    /// This is used for matching ressources against rooms defined in the config.
    churchtools_id: i64,
    /// The booking starts at...
    /// ALL DATETIMES ARE UTC.
    start_time: chrono::DateTime<Utc>,
    /// The booking ends at...
    end_time: chrono::DateTime<Utc>,
}

enum InShutdown {
    Yes,
    No,
}

async fn signal_handler(mut watcher: tokio::sync::watch::Receiver<InShutdown>, shutdown_tx: tokio::sync::watch::Sender<InShutdown>) {
    // wait for a shutdown signal
    tokio::select! {
        // shutdown the signal handler when some other process signals a shutdown
        _ = watcher.changed() => {}
        // TODO: also shutdown on SIGINT, SIGABRT and so on
        x = tokio::signal::ctrl_c() =>  {
            match x {
                Ok(()) => {
                    info!("Received Ctrl-c. Shutting down.");
                    shutdown_tx.send_replace(InShutdown::Yes);
                }
                Err(err) => {
                    error!("Unable to listen for shutdown signal: {}", err);
                    // we also shut down in case of error
                    shutdown_tx.send_replace(InShutdown::Yes);
                }
            }
        }
    }
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(config::Config::create().await?);
    // Setup tracing

    let my_crate_filter = EnvFilter::new("ct_ta_sync");
    let level_filter = filter::LevelFilter::from_str(&config.global.log_level)?;
    let subscriber = tracing_subscriber::registry().with(my_crate_filter).with(
        tracing_subscriber::fmt::layer()
            .compact()
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .with_line_number(true)
            .with_filter(level_filter),
    );
    tracing::subscriber::set_global_default(subscriber).expect("static tracing config");

    // migrate the database
    sqlx::migrate!().run(&config.db).await?;

    // cancellation token for the two main processes
    let cancel_token = CancellationToken::new();

    // the external temperature
    let external_temperature = Arc::new(RwLock::new(None));

    // cancellation channel
    let (tx, rx) = tokio::sync::watch::channel(InShutdown::No);

    // start the data-gatherer
    let gatherer_handle = tokio::spawn(pull_from_ct::keep_db_up_to_date(
        config.clone(),
        rx,
    ));

    // start the data-sender
    let emitter_handle = tokio::spawn(push_to_ta::push_coe(
        config.clone(),
        tx.subscribe(),
        external_temperature.clone(),
    ));

    // start the temperature-receiver
    let receiver_handle = tokio::spawn(read_ext_temp::read_ext_temp(
        config.clone(),
        external_temperature,
        tx.subscribe(),
        tx.clone(),
    ));

    // start the Signal handler
    let signal_handle = tokio::spawn(signal_handler(tx.subscribe(), tx.clone()));

    // Join both tasks
    let (gather_res, emit_res, receive_res, signal_res,) = tokio::join!(gatherer_handle, emitter_handle, receiver_handle, signal_handle);
    gather_res?;
    emit_res?;
    receive_res??;
    signal_res?;

    Ok(())
}
