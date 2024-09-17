//! All the db-related functions

use chrono::{format::StrftimeItems, DateTime, Local, NaiveDateTime};
use sqlx::{Pool, Sqlite};

use crate::Booking;

/// sqlite does not have tz-aware types, so we can only get NaiveDateTime from it.
/// We ALWAYS STORE UTC DATETIMES IN SQLITE.
struct NaiveBooking {
    churchtools_id: i64,
    start_time: chrono::NaiveDateTime,
    end_time: chrono::NaiveDateTime,
}
impl NaiveBooking {
    /// Taking a naive booking, interpret all datetimes as UTC datetimes
    fn interpret_as_utc(self) -> crate::Booking {
        Booking {
            churchtools_id: self.churchtools_id,
            start_time: self.start_time.and_utc(),
            end_time: self.end_time.and_utc(),
        }
    }
}

#[derive(Debug)]
pub enum DBError {
    CannotSelectBookings(sqlx::Error),
    CannotInsertBooking(sqlx::Error),
    CannotDeleteBooking(sqlx::Error),
    CannotUpdateBooking(sqlx::Error),
}
impl std::fmt::Display for DBError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::CannotSelectBookings(e) => {
                write!(
                    f,
                    "Unable to select bookings from the DB. Inner Error: {e}."
                )
            }
            Self::CannotInsertBooking(e) => {
                write!(f, "Unable to insert booking into the DB. Inner Error: {e}.")
            }
            Self::CannotUpdateBooking(e) => {
                write!(f, "Unable to update booking in the DB. Inner Error: {e}.")
            }
            Self::CannotDeleteBooking(e) => {
                write!(f, "Unable to delete booking from the DB. Inner Error: {e}.")
            }
        }
    }
}
impl std::error::Error for DBError {}

pub async fn get_all_bookings(db: &Pool<Sqlite>) -> Result<Vec<Booking>, DBError> {
    Ok(sqlx::query_as!(
        NaiveBooking,
        "SELECT churchtools_id, start_time, end_time FROM bookings;"
    )
    .fetch_all(db)
    .await
    .map_err(|e| DBError::CannotSelectBookings(e))?
    .into_iter()
    .map(|x| x.interpret_as_utc())
    .collect::<Vec<_>>())
}

/// Get all bookings in the db which intersect the interval [start, end]
pub async fn get_bookings_in_timeframe(
    db: &Pool<Sqlite>,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<Booking>, DBError> {
    let fmt = StrftimeItems::new("%Y-%m-%dT%H:%M:%S");
    let start_str = start.format_with_items(fmt.clone()).to_string();
    let end_str = end.format_with_items(fmt.clone()).to_string();
    Ok(sqlx::query_as!(
        NaiveBooking,
        "SELECT churchtools_id, start_time, end_time FROM bookings \
         WHERE start_time <= ? AND ? <= end_time;",
        end_str,
        start_str,
    )
    .fetch_all(db)
    .await
    .map_err(|e| DBError::CannotSelectBookings(e))?
    .into_iter()
    .map(|x| x.interpret_as_utc())
    .collect::<Vec<_>>())
}

/// Insert a booking into the DB
pub async fn insert_booking(db: &Pool<Sqlite>, booking: &Booking) -> Result<(), DBError> {
    let fmt = StrftimeItems::new("%Y-%m-%dT%H:%M:%S");
    let start_str = booking.start_time.format_with_items(fmt.clone()).to_string();
    let end_str = booking.end_time.format_with_items(fmt.clone()).to_string();
    sqlx::query!(
        "INSERT INTO bookings (churchtools_id, start_time, end_time) VALUES \
        (?, ?, ?);
        ",
        booking.churchtools_id,
        start_str,
        end_str,
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|e| DBError::CannotInsertBooking(e))
}

pub async fn insert_bookings<'a, I: Iterator<Item = &'a Booking>>(
    db: &Pool<Sqlite>,
    bookings: I,
) -> Result<(), DBError> {
    for b in bookings {
        insert_booking(db, b).await?;
    }
    Ok(())
}

pub async fn delete_booking(db: &Pool<Sqlite>, booking_id: i64) -> Result<(), DBError> {
    sqlx::query!(
        "DELETE FROM bookings \
        WHERE churchtools_id = ?;
        ",
        booking_id,
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|e| DBError::CannotDeleteBooking(e))
}

pub async fn delete_bookings<'a, I: Iterator<Item = i64>>(
    db: &Pool<Sqlite>,
    bookings: I,
) -> Result<(), DBError> {
    for b in bookings {
        delete_booking(db, b).await?;
    }
    Ok(())
}

pub async fn update_booking(db: &Pool<Sqlite>, booking: &Booking) -> Result<(), DBError> {
    let fmt = StrftimeItems::new("%Y-%m-%dT%H:%M:%S");
    let start_time = booking.start_time.format_with_items(fmt.clone()).to_string();
    let end_time = booking.end_time.format_with_items(fmt).to_string();
    sqlx::query!(
        "UPDATE bookings SET start_time = ?, end_time = ? \
        WHERE churchtools_id = ?;
        ",
        start_time,
        end_time,
        booking.churchtools_id,
    )
    .execute(db)
    .await
    .map(|_| ())
    .map_err(|e| DBError::CannotUpdateBooking(e))
}

pub async fn update_bookings<'a, I: Iterator<Item = &'a Booking>>(
    db: &Pool<Sqlite>,
    bookings: I,
) -> Result<(), DBError> {
    for b in bookings {
        update_booking(db, b).await?;
    }
    Ok(())
}

/// Delete all bookings from the DB which have ended in the past.
pub async fn prune_old_bookings(db: &Pool<Sqlite>) -> Result<(), DBError> {
    let time = chrono::Utc::now().naive_utc();
    let fmt = StrftimeItems::new("%Y-%m-%dT%H:%M:%S");
    let time_str = time.format_with_items(fmt).to_string();
    sqlx::query!("DELETE FROM bookings where end_time < ?;", time_str,)
        .execute(db)
        .await
        .map(|_| ())
        .map_err(|e| DBError::CannotDeleteBooking(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::NaiveDate;
    use sqlx::SqlitePool;

    #[sqlx::test(fixtures("001_good_data"))]
    async fn select_all_bookings(pool: SqlitePool) {
        let bookings = get_all_bookings(&pool).await.unwrap();
        assert_eq!(bookings.len(), 2);
        assert_eq!(
            bookings[0],
            Booking {
                churchtools_id: 123,
                start_time: DateTime::parse_from_rfc3339("2021-03-26T15:30:00+00:00")
                    .unwrap()
                    .into(),
                end_time: DateTime::parse_from_rfc3339("2021-03-26T17:00:00+00:00")
                    .unwrap()
                    .into(),
            }
        );
        assert_eq!(
            bookings[1],
            Booking {
                churchtools_id: 125,
                start_time: DateTime::parse_from_rfc3339("2021-03-28T15:30:00+00:00")
                    .unwrap()
                    .into(),
                end_time: DateTime::parse_from_rfc3339("2021-03-28T17:00:00+00:00")
                    .unwrap()
                    .into(),
            }
        );
    }

    #[sqlx::test(fixtures("001_good_data"))]
    async fn select_bookings_in_timeframe(pool: SqlitePool) {
        let start = NaiveDate::from_ymd_opt(2021, 3, 26)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let end = NaiveDate::from_ymd_opt(2021, 3, 26)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap();
        let bookings = get_bookings_in_timeframe(&pool, start, end).await.unwrap();
        assert_eq!(bookings.len(), 1);
        assert_eq!(
            bookings[0],
            Booking {
                churchtools_id: 123,
                start_time: DateTime::parse_from_rfc3339("2021-03-26T15:30:00+00:00")
                    .unwrap()
                    .into(),
                end_time: DateTime::parse_from_rfc3339("2021-03-26T17:00:00+00:00")
                    .unwrap()
                    .into(),
            }
        );
    }

    #[sqlx::test(fixtures("001_good_data"))]
    async fn delete_single_booking(pool: SqlitePool) {
        delete_booking(&pool, 123).await.unwrap();

        let start = NaiveDate::from_ymd_opt(2021, 3, 26)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let end = NaiveDate::from_ymd_opt(2021, 3, 26)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap();
        let bookings = get_bookings_in_timeframe(&pool, start, end).await.unwrap();
        assert_eq!(bookings.len(), 0);
    }

    #[sqlx::test(fixtures("001_good_data"))]
    async fn delete_multiple_bookings(pool: SqlitePool) {
        let to_delete = vec![123, 125];
        delete_bookings(&pool, to_delete.into_iter()).await.unwrap();

        let bookings = get_all_bookings(&pool).await.unwrap();
        assert_eq!(bookings.len(), 0);
    }

    #[sqlx::test(fixtures("001_good_data"))]
    async fn test_update_booking(pool: SqlitePool) {
        let new_booking = Booking {
            churchtools_id: 123,
            start_time: DateTime::parse_from_rfc3339("2021-04-26T15:30:00+00:00")
                .unwrap()
                .into(),
            end_time: DateTime::parse_from_rfc3339("2021-04-26T17:00:00+00:00")
                .unwrap()
                .into(),
        };
        update_booking(&pool, &new_booking).await.unwrap();
        let start = NaiveDate::from_ymd_opt(2021, 4, 20)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let end = NaiveDate::from_ymd_opt(2021, 5, 30)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap();
        let bookings = get_bookings_in_timeframe(&pool, start, end).await.unwrap();
        assert_eq!(bookings.len(), 1);
        assert_eq!(bookings[0], new_booking);
    }

    #[sqlx::test(fixtures("001_good_data"))]
    async fn test_insert_booking(pool: SqlitePool) {
        let new_booking = Booking {
            churchtools_id: 12341234,
            start_time: DateTime::parse_from_rfc3339("2019-04-26T14:28:00+00:00")
                .unwrap()
                .into(),
            end_time: DateTime::parse_from_rfc3339("2019-04-26T18:00:00+00:00")
                .unwrap()
                .into(),
        };
        insert_booking(&pool, &new_booking).await.unwrap();
        let start = NaiveDate::from_ymd_opt(2019, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let end = NaiveDate::from_ymd_opt(2019, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 59)
            .unwrap();
        let bookings = get_bookings_in_timeframe(&pool, start, end).await.unwrap();
        assert_eq!(bookings.len(), 1);
        assert_eq!(bookings[0], new_booking);
    }
}
