use std::{collections::HashMap, fs::File, path::Path};

use chrono::{DateTime, TimeDelta, Utc};
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use tracing::{event, Level};

#[derive(Debug)]
pub enum CreateConfigError {
    RoomNotFoundError(String),
    PDOIndexOutOfBounds(u8),
}
impl std::fmt::Display for CreateConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::RoomNotFoundError(x) => {
                write!(
                    f,
                    "Room {x} was not found in the `rooms:` section of the config."
                )
            }
            Self::PDOIndexOutOfBounds(x) => {
                write!(f, "PDO Index {x} is not within 1-64")
            }
        }
    }
}
impl std::error::Error for CreateConfigError {}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigData {
    pub cmis: Vec<CMIConfigData>,
    pub external_temperature_sensor: ExtTempConfig,
    pub ct: ChurchToolsConfig,
    pub global: GlobalConfig,
    pub rooms: HashMap<String, RoomConfig>,
}
#[derive(Debug)]
pub(crate) struct Config {
    pub cmis: Vec<CMIConfig>,
    pub external_temperature_sensor: ExtTempConfig,
    pub ct: ChurchToolsConfig,
    pub db: Pool<Sqlite>,
    pub global: GlobalConfig,
}
impl Config {
    async fn from_config_data(cd: ConfigData) -> Result<Config, Box<dyn std::error::Error>> {
        let connect_options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(crate::BOOKING_DATABASE_NAME)
            .create_if_missing(true);
        let db = sqlx::SqlitePool::connect_with(connect_options).await?;

        let cmis = cd
            .cmis
            .into_iter()
            .map(|cmi| {
                Ok::<CMIConfig, CreateConfigError>(CMIConfig {
                    host: cmi.host,
                    our_virtual_can_id: cmi.our_virtual_can_id,
                    rooms: cmi
                        .rooms
                        .into_iter()
                        .map(|room| {
                            let room_data = cd
                                .rooms
                                .get(&room.name)
                                .ok_or(CreateConfigError::RoomNotFoundError(room.name.clone()))?;
                            Ok(AssociatedRoomConfig {
                                pdo_index: if room.pdo_index >= 1 && room.pdo_index <= 64 {
                                    room.pdo_index - 1
                                } else {
                                    return Err(CreateConfigError::PDOIndexOutOfBounds(
                                        room.pdo_index,
                                    ));
                                },
                                churchtools_id: room_data.churchtools_id,
                                preheat_minutes: room_data.preheat_minutes.unwrap_or(30),
                                preshutdown_minutes: room_data.preshutdown_minutes.unwrap_or(10),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // shift the pdo_offset for the external_temperature_sensor data by one:
        let ext_temp_config = ExtTempConfig {
            bind_addr: cd.external_temperature_sensor.bind_addr,
            can_id: cd.external_temperature_sensor.can_id,
            pdo_index: if (1..=64).contains(&cd.external_temperature_sensor.pdo_index) {
                cd.external_temperature_sensor.pdo_index - 1
            } else {
                return Err(Box::new(CreateConfigError::PDOIndexOutOfBounds(
                    cd.external_temperature_sensor.pdo_index,
                )));
            },
            timeout: cd.external_temperature_sensor.timeout,
        };

        Ok(Config {
            cmis,
            external_temperature_sensor: ext_temp_config,
            ct: cd.ct,
            db,
            global: cd.global,
        })
    }

    pub async fn create() -> Result<Config, Box<dyn std::error::Error>> {
        let path = Path::new("/etc/ct-ta-sync/config.yaml");
        let f = match File::open(path) {
            Ok(x) => x,
            Err(e) => {
                event!(
                    Level::ERROR,
                    "config file /etc/asterconf/config.yaml not readable: {e}"
                );
                return Err(Box::new(e));
            }
        };
        let config_data: ConfigData = match serde_yaml::from_reader(f) {
            Ok(x) => x,
            Err(e) => {
                event!(Level::ERROR, "config file had syntax errors: {e}");
                return Err(Box::new(e));
            }
        };
        Config::from_config_data(config_data).await
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct RoomConfig {
    pub preheat_minutes: Option<u8>,
    pub preshutdown_minutes: Option<u8>,
    pub churchtools_id: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlobalConfig {
    pub ct_pull_frequency: u64,
    pub ta_push_frequency: u64,
    pub log_level: String,
    pub emiter_bind_addr: String,
}

#[derive(Debug)]
pub(crate) struct CMIConfig {
    pub host: String,
    pub our_virtual_can_id: u8,
    pub rooms: Vec<AssociatedRoomConfig>,
}

#[derive(Debug)]
pub(crate) struct AssociatedRoomConfig {
    pub churchtools_id: i64,
    pub pdo_index: u8,
    pub preheat_minutes: u8,
    pub preshutdown_minutes: u8,
}
impl AssociatedRoomConfig {
    /// Calculate the amount of minutes a room should be preheated, depending on the the
    /// base_preheating time set in the config and the external temperature
    ///
    /// external temperature is expected in tenths of a Degree Centigrade
    /// if external_temp is None, we do not scale the base shutdowns at all.
    fn preheat_time(&self, external_temp: Option<i32>) -> u8 {
        if let Some(x) = external_temp {
            let clamped_external_temp: f64 = x.clamp(-100, 200) as f64;
            let time_proportion = (clamped_external_temp + 100_f64) / 300_f64;
            (self.preheat_minutes as f64 * (1_f64 - time_proportion)).round() as u8
        } else {
            self.preheat_minutes
        }
    }

    /// Calculate the amount of minutes a rooms heating may be shut down BEFORE the end of a booking
    /// base_preshutdown time set in the config and the external temperature
    ///
    /// external temperature is expected in tenths of a Degree Centigrade
    /// if external_temp is None, we do not scale the base shutdowns at all.
    fn preshutdown_time(&self, external_temp: Option<i32>) -> u8 {
        if let Some(x) = external_temp {
            let clamped_external_temp: f64 = x.clamp(-100, 200) as f64;
            let time_proportion = (clamped_external_temp + 100_f64) / 300_f64;
            (self.preshutdown_minutes as f64 * time_proportion).round() as u8
        } else {
            // if we do not now how warm it is, we are never allowed to prematurely stop heating
            0
        }
    }

    /// Apply both prehead and preshutdown times, depending on this rooms configuration.
    /// Return the real start and real end time (i.e. the times where we have to start heating or
    /// are allowed to stop heating).
    ///
    /// external temperature is expected in tenths of a Degree Centigrade
    /// if external_temp is None, we do not scale the base shutdowns at all.
    pub fn apply_preheat_and_preshutdown(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        external_temp: Option<i32>,
    ) -> (DateTime<Utc>, DateTime<Utc>) {
        let new_start = start - TimeDelta::minutes(self.preheat_time(external_temp).into());
        let new_end = end - TimeDelta::minutes(self.preshutdown_time(external_temp).into());
        (new_start, new_end)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CMIConfigData {
    pub host: String,
    pub our_virtual_can_id: u8,
    pub rooms: Vec<AssociatedRoomConfigData>,
}

/// a single room defined in the config
#[derive(Debug, Deserialize)]
pub(crate) struct AssociatedRoomConfigData {
    name: String,
    pub pdo_index: u8,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExtTempConfig {
    /// IP Address to bind a receiving UDP socket on. Port is 5442
    pub bind_addr: String,
    /// Can ID to expect - other ids are ignored
    pub can_id: u8,
    /// PDO Index to expect - other ids are ignored
    pub pdo_index: u8,
    /// number of minutes to wait for a packet to the correct Can-ID, PDO.
    /// After this time, the external temperature is not considered anymore
    pub timeout: u8,
}

#[derive(Deserialize)]
pub(crate) struct ChurchToolsConfig {
    pub host: String,
    pub login_token: String,
}
impl std::fmt::Debug for ChurchToolsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("ChurchToolsConfig")
            .field("host", &self.host)
            .field("login_token", &"[redacated]")
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn preheat_time_below_start() {
        let external_temp = -200;
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preheat_time(Some(external_temp)), 40);
    }

    #[test]
    fn preheat_time_ext_unknown() {
        let external_temp = None;
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preheat_time(external_temp), 40);
    }

    #[test]
    fn preheat_time_ext_high() {
        let external_temp = Some(200);
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preheat_time(external_temp), 0);
    }

    #[test]
    fn preheat_time_ext_middle() {
        let external_temp = Some(50);
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preheat_time(external_temp), 20);
    }

    #[test]
    fn preshutdown_time_below_start() {
        let external_temp = -200;
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preshutdown_time(Some(external_temp)), 0);
    }

    #[test]
    fn preshutdown_time_ext_unknown() {
        let external_temp = None;
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preshutdown_time(external_temp), 0);
    }

    #[test]
    fn preshutdown_time_ext_high() {
        let external_temp = Some(200);
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preshutdown_time(external_temp), 13);
    }

    #[test]
    fn preshutdown_time_ext_middle() {
        let external_temp = Some(50);
        let room = AssociatedRoomConfig {
            churchtools_id: 0,
            pdo_index: 0,
            preheat_minutes: 40,
            preshutdown_minutes: 13,
        };
        assert_eq!(room.preshutdown_time(external_temp), 7);
    }
}
