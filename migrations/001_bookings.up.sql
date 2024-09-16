-- UP booking table
CREATE TABLE bookings (
	churchtools_id INTEGER PRIMARY KEY,
	room TEXT NOT NULL,
	start_time DATETIME NOT NULL,
	end_time DATETIME NOT NULL
);

