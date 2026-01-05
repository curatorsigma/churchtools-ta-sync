//! Get data from Churchtools

use std::{str::FromStr, sync::Arc};

use chrono::{TimeZone, Utc};
use itertools::Itertools;
use reqwest::header;
use serde::Deserialize;
use tracing::{debug, info, trace, warn};

use crate::{config::Config, db::DBError, Booking, InShutdown};
// ignore bookings with this string in their description
pub(crate) const IGNORE_MAGIC_STRING: &str = "NICHT_HEIZEN";

#[derive(Debug, Deserialize)]
struct CTBookingsResponse {
    data: Vec<BookingsData>,
}
#[derive(Debug, Deserialize)]
struct BookingsData {
    base: BookingsDataBase,
    calculated: BookingsDataCalculated,
}

#[derive(Debug, Deserialize)]
struct BookingsDataBase {
    /// this is the bookings ID
    id: i64,
    resource: ResourceData,
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResourceData {
    /// this is the resources ID
    id: i64,
}

#[derive(Debug, Deserialize)]
struct BookingsDataCalculated {
    #[serde(rename = "startDate")]
    start_date: String,
    #[serde(rename = "endDate")]
    end_date: String,
}

#[derive(Debug)]
pub enum CTApiError {
    GetBookings(reqwest::Error),
    Deserialize,
    Utf8Decode,
    ParseTime(String),
}
impl std::fmt::Display for CTApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::GetBookings(e) => {
                write!(f, "Cannot get bookings. reqwest Error: {e}")
            }
            Self::Deserialize => {
                write!(f, "Cannot deserialize the response.")
            }
            Self::Utf8Decode => {
                write!(f, "Cannot decode the message bytes as utf-8.")
            }
            Self::ParseTime(x) => {
                write!(
                    f,
                    "Cannot parse a date or time contained in CTs response. Input: {x}."
                )
            }
        }
    }
}
impl std::error::Error for CTApiError {}

/// Something went wrong while gathering Information from CT into the DB
#[derive(Debug)]
pub enum GatherError {
    DB(crate::db::DBError),
    CT(CTApiError),
}
impl std::fmt::Display for GatherError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::DB(x) => write!(f, "DBError: {x}"),
            Self::CT(x) => write!(f, "CTApiError: {x}"),
        }
    }
}
impl std::error::Error for GatherError {}
impl From<DBError> for GatherError {
    fn from(value: DBError) -> Self {
        Self::DB(value)
    }
}
impl From<CTApiError> for GatherError {
    fn from(value: CTApiError) -> Self {
        Self::CT(value)
    }
}

enum CalculateFor {
    Start,
    End,
}

/// CT sometimes sends RFC3339, and sometimes YYYY-MM-DD only
///
/// `start_or_end`: Do we set the time to 00:00:00 (Start)or 23:59:59 (End) when only a Date is
/// given?
fn convert_date_or_datetime(
    input: &str,
    start_or_end: CalculateFor,
) -> Option<chrono::DateTime<Utc>> {
    match chrono::DateTime::parse_from_rfc3339(input) {
        Ok(x) => Some(x.into()),
        Err(_) => match chrono::NaiveDate::from_str(input) {
            Ok(y) => match start_or_end {
                CalculateFor::Start => Some(
                    chrono::Utc
                        .from_local_datetime(
                            &y.and_hms_opt(0, 0, 0).expect("Statically good time."),
                        )
                        .earliest()
                        .expect("No DST at 00:00:00"),
                ),
                CalculateFor::End => Some(
                    chrono::Utc
                        .from_local_datetime(
                            &y.and_hms_opt(23, 59, 59).expect("Statically good time."),
                        )
                        .earliest()
                        .expect("No DST at 23:59:59"),
                ),
            },
            Err(_) => None,
        },
    }
}

/// Actually get Booking Data from Churchtools
async fn get_relevant_bookings(
    config: &Config,
    client: &reqwest::Client,
    start_date: chrono::NaiveDate,
    end_date: chrono::NaiveDate,
) -> Result<Vec<Booking>, CTApiError> {
    let mut query_strings = config
        .cmis
        .iter()
        .flat_map(|cmi| &cmi.rooms)
        .map(|room_config| room_config.room_config.churchtools_id)
        .unique()
        // we now have the resource ids we care about
        // convert them to the query parameters we need
        .map(|id| ("resource_ids[]", format!("{id}")))
        .collect::<Vec<_>>();
    // use bookingsin the relevant timeframe
    query_strings.push(("from", start_date.to_string()));
    query_strings.push(("to", end_date.to_string()));
    // use bookings that are
    // --- pending
    query_strings.push(("status_ids[]", "1".to_owned()));
    // --- approved
    query_strings.push(("status_ids[]", "2".to_owned()));
    let response = match client
        .get(format!("https://{}/api/bookings", config.ct.host))
        .query(&query_strings)
        .send()
        .await
    {
        Ok(x) => {
            let text_res = x.text().await;
            match text_res {
                Ok(text) => {
                    let deser_res: Result<CTBookingsResponse, _> = serde_json::from_str(&text);
                    if let Ok(y) = deser_res {
                        y
                    } else {
                        warn!("There was an error parsing the return value from CT.");
                        warn!("The complete text received was: {text}");
                        return Err(CTApiError::Deserialize);
                    }
                }
                Err(e) => {
                    warn!("There was an error reading the response from CT as utf-8: {e}");
                    return Err(CTApiError::Utf8Decode);
                }
            }
        }
        Err(e) => {
            warn!("There was a problem getting a response from CT");
            return Err(CTApiError::GetBookings(e));
        }
    };
    response
        .data
        .into_iter()
        // ignore bookings that are forced ignored for heating
        .filter(|x: &BookingsData| {
            !x.base
                .note
                .as_ref()
                .is_some_and(|note| note.contains(IGNORE_MAGIC_STRING))
        })
        .map(|x: BookingsData| {
            Ok::<Booking, CTApiError>(Booking {
                booking_id: x.base.id,
                resource_id: x.base.resource.id,
                start_time: convert_date_or_datetime(&x.calculated.start_date, CalculateFor::Start)
                    .ok_or(CTApiError::ParseTime(x.calculated.start_date))?
                    // we get the date from CT with an unknown offset, and need to cast to UTC
                    // (actually, CT seems to always return UTC, but this is not part of a stably documented API)
                    .into(),
                end_time: convert_date_or_datetime(&x.calculated.end_date, CalculateFor::End)
                    .ok_or(CTApiError::ParseTime(x.calculated.end_date))?
                    .into(),
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

/// Read Bookings from CT and import them into the DB
async fn get_bookings_into_db(config: Arc<Config>, client: &reqwest::Client) -> Result<(), GatherError> {
    let start = Utc::now().naive_utc().into();
    let end = start + chrono::TimeDelta::days(1);
    // get bookings from CT
    let bookings_from_ct = get_relevant_bookings(&config, client, start, end).await?;
    // get bookings from db
    let bookings_from_db = crate::db::get_bookings_in_timeframe(
        &config.db,
        start.and_time(chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("statically good time")),
        end.and_time(chrono::NaiveTime::from_hms_opt(23, 59, 59).expect("statically good time")),
    )
    .await?;

    // compare the two sources
    // add new bookings
    trace!("in db: {bookings_from_db:?}");
    trace!("in ct: {bookings_from_ct:?}");
    let new_bookings = bookings_from_ct.iter().filter(|b| {
        !bookings_from_db
            .iter()
            .any(|x| x.booking_id == b.booking_id)
    });
    trace!(
        "Adding these bookings: {:?}",
        new_bookings.clone().collect::<Vec<_>>()
    );
    crate::db::insert_bookings(&config.db, new_bookings).await?;

    // remove bookings no longer present in ct
    let deprecated_bookings = bookings_from_db
        .iter()
        .map(|b| b.booking_id)
        .filter(|&id| !bookings_from_ct.iter().any(|x| x.booking_id == id));
    crate::db::delete_bookings(&config.db, deprecated_bookings).await?;

    // Update bookings that have changed times in CT
    let changed_bookings = bookings_from_ct.iter().filter(|b| {
        bookings_from_db
            .iter()
            .any(|x| x.booking_id == b.booking_id && x != *b)
    });
    crate::db::update_bookings(&config.db, changed_bookings).await?;
    Ok(())
}

/// Create a Client with cookie store that sends the correct auth header each time
///
/// CT will honor the session cookie, and relogin when the cookie is stable because the correct
/// auth header is also sent.
fn create_client(config: &Config) -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::ACCEPT, header::HeaderValue::from_static("application/json"));
    let mut auth_value = header::HeaderValue::from_str(&format!("Login {}",config.ct.login_token)).expect("statically good header");
    auth_value.set_sensitive(true);
    headers.insert(header::AUTHORIZATION, auth_value);
    reqwest::Client::builder().cookie_store(true).default_headers(headers).use_rustls_tls().build()
}

/// Continuously pull Data from CT into the DB
pub async fn keep_db_up_to_date(
    config: Arc<Config>,
    mut watcher: tokio::sync::watch::Receiver<InShutdown>,
    shutdown_tx: tokio::sync::watch::Sender<InShutdown>,
) {
    info!("Starting CT -> DB Sync task");
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        config.global.ct_pull_frequency,
    ));

    let client = match create_client(&config) {
        Ok(x) => x,
        Err(e) => {
            tracing::error!("Unable to create reqwest client: {e}");
            shutdown_tx.send_replace(InShutdown::Yes);
            return;
        }
    };

    interval.tick().await;
    loop {
        debug!("Gatherer starting new run.");
        // get new data
        let ct_to_db_res = get_bookings_into_db(config.clone(), &client).await;
        match ct_to_db_res {
            Ok(()) => debug!("Successfully updated db."),
            Err(e) => {
                warn!("Failed to update db from CT. Error encountered: {e}");
            }
        };
        // prune old entries in db
        let db_prune_res = crate::db::prune_old_bookings(&config.db).await;
        match db_prune_res {
            Ok(x) => match x {
                0 => debug!("Successfully pruned db. Removed {x} old bookings."),
                y => info!("Successfully pruned db. Removed {y} old bookings."),
            },
            Err(e) => {
                warn!("Failed to prune db. Error encountered: {e}");
            }
        };
        // stop on cancellation or continue after the next tick
        tokio::select! {
            _ = watcher.changed() => {
                debug!("Shutting down data gatherer now.");
                return;
            }
            _ = interval.tick() => {}
        }
    }
}

#[cfg(test)]
mod test {
    use chrono::TimeZone;

    use super::convert_date_or_datetime;

    #[test]
    fn date_parsing() {
        let input = "2025-01-25";
        let parsed = convert_date_or_datetime(input, super::CalculateFor::Start).unwrap();
        assert_eq!(
            parsed,
            chrono::Utc.with_ymd_and_hms(2025, 1, 25, 0, 0, 0).unwrap()
        );

        let input = "2025-01-25";
        let parsed = convert_date_or_datetime(input, super::CalculateFor::End).unwrap();
        assert_eq!(
            parsed,
            chrono::Utc
                .with_ymd_and_hms(2025, 1, 25, 23, 59, 59)
                .unwrap()
        );
    }

    #[test]
    fn datetime_parsing() {
        let input = "2025-01-25T12:30:11Z";
        let parsed = convert_date_or_datetime(input, super::CalculateFor::Start).unwrap();
        assert_eq!(
            parsed,
            chrono::Utc
                .with_ymd_and_hms(2025, 1, 25, 12, 30, 11)
                .unwrap()
        );

        let input = "2025-01-25T10:00:00+01:00";
        let parsed = convert_date_or_datetime(input, super::CalculateFor::End).unwrap();
        assert_eq!(
            parsed,
            chrono::Utc.with_ymd_and_hms(2025, 1, 25, 9, 0, 0).unwrap()
        );
    }
}
