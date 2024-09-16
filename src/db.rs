//! All the db-related functions

use chrono::{DateTime, Local, NaiveDateTime};
use sqlx::{Pool, Sqlite};

use crate::Booking;

#[derive(Debug)]
pub enum DBError {
    CannotSelectBookings(sqlx::Error),
    CannotInsertBooking(sqlx::Error),
}
impl std::fmt::Display for DBError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::CannotSelectBookings(e) => {
                write!(f, "Unable to select bookings from the DB. Inner Error: {e}.")
            }
            Self::CannotInsertBooking(e) => {
                write!(f, "Unable to insert bookings into the DB. Inner Error: {e}.")
            }
        }
    }
}
impl std::error::Error for DBError {}

pub async fn get_all_bookings(db: &Pool<Sqlite>) -> Result<Vec<Booking>, DBError> {
    sqlx::query_as!(crate::Booking, "SELECT churchtools_id, start_time, end_time FROM bookings;")
        .fetch_all(db).await.map_err(|e| DBError::CannotSelectBookings(e))
}

pub async fn get_bookings_in_timeframe(db: &Pool<Sqlite>, start: NaiveDateTime, end: NaiveDateTime) -> Result<Vec<Booking>, DBError>  {
    sqlx::query_as!(crate::Booking,
        "SELECT churchtools_id, start_time, end_time FROM bookings \
         WHERE start_time > ? AND end_time < ?;",
         start,
         end
        )
        .fetch_all(db).await.map_err(|e| DBError::CannotSelectBookings(e))
}

pub async fn insert_booking(db: &Pool<Sqlite>, booking: Booking) -> Result<(), DBError> {
    sqlx::query!(
        "INSERT INTO bookings (churchtools_id, start_time, end_time) VALUES \
        (?, ?, ?);
        ",
            booking.churchtools_id, 
            booking.start_time,
            booking.end_time,
        )
        .execute(db).await
        .map(|_| ())
        .map_err(|e| DBError::CannotInsertBooking(e))
}

pub async fn prune_old_bookings(db: &Pool<Sqlite>) -> Result<(), DBError> {
    let time = chrono::Utc::now().naive_utc();
    sqlx::query!(
        "DELETE FROM bookings where end_time < ?;",
        time,
        )
        .execute(db).await
        .map(|_| ())
        .map_err(|e| DBError::CannotInsertBooking(e))
}

