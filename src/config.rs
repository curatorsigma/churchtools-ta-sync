use std::{fs::File, path::Path};

use serde::Deserialize;
use sqlx::{Pool, Sqlite};
use tracing::{event, Level};

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigData {
    pub cmis: Vec<CMIConfig>,
    pub external_temperature_sensor: ExtTempConfig,
    pub ct: ChurchToolsConfig,
    pub global: GlobalConfig,
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
    async fn from_config_data(cd: ConfigData) -> Result<Config, sqlx::Error> {
        let connect_options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(crate::BOOKING_DATABASE_NAME)
            .create_if_missing(true);
        let db = sqlx::SqlitePool::connect_with(connect_options).await?;
        Ok(Config {
            cmis: cd.cmis,
            external_temperature_sensor: cd.external_temperature_sensor,
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
        Ok(Config::from_config_data(config_data).await?)
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct GlobalConfig {
  pub ct_pull_frequency: u64,
  pub ta_push_frequency: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CMIConfig {
    pub host: String,
    pub our_virtual_can_id: u8,
    pub rooms: Vec<RoomConfig>,
}

/// a single room defined in the config
#[derive(Debug, Deserialize)]
pub(crate) struct RoomConfig {
    /// the corresponsding resource id in churchtools
    /// i64 for compatibility with the sqlite DB
    pub churchtools_id: i64,
    pub name: String,
    pub pdo_index: u8,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ExtTempConfig {
    bind_addr: String,
    can_id: u8,
    pdo_index: u8,
    timeout: u8,
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
