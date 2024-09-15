//! All the db-related functions

pub enum DBError {
}

pub async fn get_all_bookings() -> Result<Vec<Booking>, DBError> {
}

pub async fn get_bookings_in_timeframe(start: TodoPointInTime, stop: TodoPointInTime) -> Result<Vec<Booking>, DBError>  {
}

pub async fn insert_booking(booking: Booking) -> Result<(), DBError> {
}

pub async fn prune_old_bookings() -> Result<(), DBError> {
}

