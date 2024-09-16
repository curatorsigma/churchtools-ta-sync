-- UP booking table
CREATE TABLE bookings (
	churchtools_id INTEGER PRIMARY KEY,
	room TEXT NOT NULL,
	start_time INTEGER NOT NULL,
	stop_time INTEGER NOT NULL
);

