use std::{fs::File, path::Path};

use serde::Deserialize;
use tracing::{event, Level};

#[derive(Debug,Deserialize)]
pub(crate) struct Config {
    cmis: Vec<CMIConfig>,
    external_temperature_sensor: ExtTempConfig,
    ct: ChurchToolsConfig,
}
impl Config {
    pub fn create() -> Result<Config, Box<dyn std::error::Error>> {
        let path = Path::new("/etc/ct-ta-sync/config.yaml");
        let f = match File::open(path) {
            Ok(x) => x,
            Err(e) => {
                event!(Level::ERROR, "config file /etc/asterconf/config.yaml not readable: {e}");
                return Err(Box::new(e));
            }
        };
        let config_data: Config = match serde_yaml::from_reader(f) {
            Ok(x) => x,
            Err(e) => {
                event!(Level::ERROR, "config file had syntax errors: {e}");
                return Err(Box::new(e));
            }
        };
        Ok(config_data)
    }
}

#[derive(Debug,Deserialize)]
pub(crate) struct CMIConfig {
    host: String,
    our_virtual_can_id: u8,
    rooms: Vec<RoomConfig>,
}

#[derive(Debug,Deserialize)]
pub(crate) struct RoomConfig {
    name: String,
    pdo_index: u8,
}

#[derive(Debug,Deserialize)]
pub(crate) struct ExtTempConfig {
    bind_addr: String,
    can_id: u8,
    pdo_index: u8,
    timeout: u8,
}

#[derive(Deserialize)]
pub(crate) struct ChurchToolsConfig {
    host: String,
    login_token: String,
}
impl std::fmt::Debug for ChurchToolsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("ChurchToolsConfig")
            .field("host", &self.host)
            .field("login_token", &"[redacated]")
            .finish()
    }
}

