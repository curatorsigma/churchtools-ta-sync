//! Get data from Churchtools

// TODO: request or some shit?

use std::sync::Arc;

use crate::config::ChurchToolsConfig;

async fn get_bookings_into_db(config: Arc<ChurchToolsConfig>) {
    // make requests
    // put stuff into db
}

async fn keep_db_up_to_date(config: Arc<ChurchToolsConfig>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(300));
    loop {
        // get new data
        // prune old entries in db
        interval.tick().await;
    }
}
