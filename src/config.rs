use std::{collections::HashMap, fs::File, path::Path};

use chrono::Duration;
use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use tracing::{event, Level};

use crate::read_ext_temp::RoomTemperatureStatus;

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
    pub current_temperature: GlobalCurrentTemperatureConfig,
    pub ct: ChurchToolsConfig,
    pub global: GlobalConfig,
    pub rooms: HashMap<String, RoomConfigData>,
}
#[derive(Debug)]
pub(crate) struct Config {
    pub cmis: Vec<CMIConfig>,
    pub current_temperature: GlobalCurrentTemperatureConfig,
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
                                name: room.name,
                                pdo_index: if room.pdo_index >= 1 && room.pdo_index <= 64 {
                                    room.pdo_index - 1
                                } else {
                                    return Err(CreateConfigError::PDOIndexOutOfBounds(
                                        room.pdo_index,
                                    ));
                                },
                                room_config: room_data.clone().try_into()?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Config {
            cmis,
            current_temperature: cd.current_temperature,
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

    /// If there is a room config whose can_id and pdo matches the inputs, return that rooms name.
    pub fn can_pdo_is_known(&self, can_id: u8, pdo: u8) -> Option<String> {
        for cmi in &self.cmis {
            for room in &cmi.rooms {
                if room.is_this_can_pdo(can_id, pdo) {
                    return Some(room.name.clone());
                }
            }
        }
        None
    }

    /// Get the timeout for a room if that room exists.
    pub fn get_timeout_by_room_name(&self, name: &str) -> Option<u8> {
        for cmi in &self.cmis {
            for room in &cmi.rooms {
                if room.name == name {
                    return Some(room.room_config.current_temperature.timeout);
                }
            }
        };
        None
    }

    /// Create the empty temperature structure for all known rooms.
    pub fn get_empty_temperature_status(&self) -> HashMap<String, RoomTemperatureStatus> {
        let mut res = HashMap::<String, RoomTemperatureStatus>::new();
        for cmi in &self.cmis {
            for room in &cmi.rooms {
                res.insert(room.name.clone(), RoomTemperatureStatus::new(room.room_config.current_temperature.timeout as u64 * 60));
            }
        }
        res
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentTemperatureConfig {
    pub receiving_can_id: u8,
    pub receiving_pdo: u8,
    pub timeout: u8,
    pub timeout_assume_temperature: Option<f32>,
}
impl TryFrom<CurrentTemperatureConfigData> for CurrentTemperatureConfig {
    type Error = CreateConfigError;
    fn try_from(value: CurrentTemperatureConfigData) -> Result<Self, Self::Error> {
        Ok(Self {
            receiving_can_id: value.receiving_can_id,
            receiving_pdo: if value.receiving_pdo >= 1 && value.receiving_pdo <= 64 {
                value.receiving_pdo - 1
            } else {
                return Err(CreateConfigError::PDOIndexOutOfBounds(
                    value.receiving_pdo,
                ));
            },
            timeout: value.timeout,
            timeout_assume_temperature: value.timeout_assume_temperature,
        })
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct CurrentTemperatureConfigData {
    pub receiving_can_id: u8,
    pub receiving_pdo: u8,
    #[serde(default = "default_timeout")]
    pub timeout: u8,
    pub timeout_assume_temperature: Option<f32>,
}
fn default_timeout() -> u8 {
    10
}

// temperature == T. t does not make sense.
#[allow(non_snake_case)]
#[derive(Debug, Clone)]
pub(crate) struct RoomConfig {
    /// CT ID of the ressource corresponding to this room
    pub churchtools_id: i64,
    /// How many K can this room be heated per h?
    pub delta_T_per_hour: f32,
    /// Which temperature in °C / 10 do we want this room to have?
    pub target_temperature: f32,
    pub current_temperature: CurrentTemperatureConfig,
}
impl TryFrom<RoomConfigData> for RoomConfig {
    type Error = CreateConfigError;
    fn try_from(value: RoomConfigData) -> Result<Self, Self::Error> {
        Ok(Self {
            churchtools_id: value.churchtools_id,
            delta_T_per_hour: value.delta_T_per_hour,
            target_temperature: value.target_temperature,
            current_temperature: value.current_temperature.try_into()?,
        })
    }
}

// temperature == T. t does not make sense.
#[allow(non_snake_case)]
#[derive(Debug, Deserialize, Clone)]
pub(crate) struct RoomConfigData {
    /// CT ID of the ressource corresponding to this room
    pub churchtools_id: i64,
    /// How many K can this room be heated per h?
    pub delta_T_per_hour: f32,
    /// Which temperature in °C / 10 do we want this room to have?
    pub target_temperature: f32,
    pub current_temperature: CurrentTemperatureConfigData,
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

/// A room associated to a receiving CMI
#[derive(Debug)]
pub(crate) struct AssociatedRoomConfig {
    pub name: String,
    pub room_config: RoomConfig,
    pub pdo_index: u8,
}
impl AssociatedRoomConfig {
    /// Return true iff this room expects its temperature to come from the given CAN-ID and PDO
    fn is_this_can_pdo(&self, can_id: u8, pdo: u8) -> bool {
        self.room_config.current_temperature.receiving_can_id == can_id && self.room_config.current_temperature.receiving_pdo == pdo
    }

    /// calculate the required time to preheat this room
    pub fn required_preheat_time(&self, current_temp: &RoomTemperatureStatus, global_assume_current_temperature_offset: f32) -> Duration {
        let effective_temperature = current_temp.current_temperature().unwrap_or(
            self.room_config.current_temperature.timeout_assume_temperature.unwrap_or(self.room_config.target_temperature - global_assume_current_temperature_offset)
        );
        let required_time_in_s = (self.room_config.target_temperature - effective_temperature).max(0.0) / self.room_config.delta_T_per_hour * 3600.0;
        Duration::seconds(required_time_in_s as i64)
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

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct GlobalCurrentTemperatureConfig {
    /// IP Address to bind a receiving UDP socket on. Port is 5442
    pub bind_addr: String,
    /// When timeout occurs for a room and no assume temperature is given for it,
    /// assume that the current temperature is [`Self::global_assume_current_temperature_offset`] below the
    /// rooms [`RoomConfig::target_temperature`].
    pub global_assume_current_temperature_offset: f32,
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
}
