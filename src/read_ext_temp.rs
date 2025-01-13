//! Read the external temperature from a CMI sending that information.

use std::{collections::HashMap, sync::Arc};

use coe::{AnalogueCOEValue, COEValue, Packet};
use tokio::{net::UdpSocket, sync::RwLock, time::Instant};
use tracing::{debug, error, info, trace, warn};

use crate::{config::Config, InShutdown};

/// Extract all known room temperatures from `packet`.
fn handle_coe_packet(config: &Config, packet: Packet) -> Vec<(String, f32)> {
    let mut known_payloads = Vec::<(String, f32)>::new();

    for payload in packet {
        // go through all rooms and see if this payload fits
        match config.can_pdo_is_known(payload.node(), payload.pdo_index()) {
            Some(room_name) => {
                if let COEValue::Analogue(
                    AnalogueCOEValue::DegreeCentigrade_Tens(x),
                ) = payload.value()
                {
                    debug!("Got the temperature {} °C for {}", x as f32 / 10_f32, room_name);
                    known_payloads.push((room_name, x as f32 / 10_f32));
                } else {
                    warn!("Got Payload for correct ID and Index, but the Unit was not Degree Centigrade ({}).", payload.unit_id());
                }
            }
            None => {
                trace!("Received payload {payload:?}. Does not fit a known room.");
            }
        }
    };
    if known_payloads.is_empty() {
        debug!("Got a well-formed COE packet, but none of its payloads fit a known room.");
    }
    known_payloads
}

/// Parse UDP packets until we receive a well-formed COE packet for a known room.
///
/// Then, return the room name and current temperature for all known payloads in it.
///
/// As a first value, return the time elapsed in seconds, truncated.
pub async fn read_next_temp_packet(sock: &UdpSocket, config: &Config) -> (u64, Vec<(String, f32)>) {
    let start_time = Instant::now();

    // all well-formed COE packets are at most 252 bytes long
    let mut buf = [0_u8; 252];
    loop {
        let bytes = sock.recv_from(&mut buf).await;
        match bytes {
            Ok(x) => {
                trace!("Received a CoE packet of {} bytes", x.0);
                let parse_res = TryInto::<Packet>::try_into(&buf[0..x.0]);
                match parse_res {
                    Ok(packet) => {
                        let room_temps = handle_coe_packet(config, packet);
                        if !room_temps.is_empty() {
                            let end_time = Instant::now();
                            let elapsed = end_time.saturating_duration_since(start_time).as_secs();
                            return (elapsed, room_temps);
                        };
                    }
                    Err(e) => {
                        warn!("Packet received, but not parsable as CoE: {e}");
                    }
                };
            }
            Err(e) => {
                trace!("Failed to read a CoE packet: {e}");
            }
        }
    }
}

#[derive(Debug)]
pub enum ReadExtTempError {
    Udp(std::io::Error),
    RoomKnownButConfigNotPresent(String),
}
impl std::fmt::Display for ReadExtTempError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Udp(x) => write!(f, "Udp Error: {x}"),
            Self::RoomKnownButConfigNotPresent(x) => write!(f, "Room {x} was known but its config is not present. Programmer Error."),
        }
    }
}
impl From<std::io::Error> for ReadExtTempError {
    fn from(value: std::io::Error) -> Self {
        Self::Udp(value)
    }
}
impl std::error::Error for ReadExtTempError {}


#[derive(Debug, Copy, Clone)]
pub(crate) struct RoomTemperatureStatus {
    /// the last temperature we got for this room
    /// in °C / 10
    last_temperature: f32,
    /// time until this room will go into timeout - in seconds
    time_till_timeout: u64,
}
impl RoomTemperatureStatus {
    pub fn default() -> Self {
        Self {
            last_temperature: 0.0,
            time_till_timeout: 0,
        }
    }

    pub fn current_temperature(&self) -> Option<f32> {
        if self.in_timeout() {
            None
        } else {
            Some(self.last_temperature)
        }
    }

    fn update_with(&mut self, name: &str, temperature: f32, config: &Config) -> Result<(), ReadExtTempError> {
        self.last_temperature = temperature;
        match config.get_timeout_by_room_name(name) {
            Some(x) => { self.time_till_timeout = x as u64 * 60; }
            None => { return Err(ReadExtTempError::RoomKnownButConfigNotPresent(name.to_owned())) }
        };
        Ok(())
    }

    fn tick_down(&mut self, elapsed: u64) {
        self.time_till_timeout = self.time_till_timeout.saturating_sub(elapsed);
    }

    pub fn in_timeout(&self) -> bool {
        self.time_till_timeout == 0
    }
}

/// Update the external temperature whenever a corresponding value is received from a CMI.
///
/// After config.external_temperature_sensor.timeout minutes, the External Temperature is set to
/// None
pub async fn read_ext_temp(
    config: Arc<Config>,
    ext_temp: Arc<RwLock<HashMap<String, RoomTemperatureStatus>>>,
    mut watcher: tokio::sync::watch::Receiver<InShutdown>,
    shutdown_tx: tokio::sync::watch::Sender<InShutdown>,
) -> Result<(), ReadExtTempError> {
    info!("Starting external temperature receiver");
    // crate Udp socket
    let sock =
        match UdpSocket::bind((config.current_temperature.bind_addr.clone(), 5442)).await {
            Ok(x) => x,
            Err(e) => {
                error!("Unable to open Udp Socket to listen for incoming external temperature.");
                shutdown_tx.send_replace(InShutdown::Yes);
                return Err(e.into());
            }
        };

    // listen for UDP packets for 1m
    // IF something was received in that time:
    // - decrease the "time_till_timeout" for each room by the time that took
    // - deal with the packet
    //     - IF that has info for a room, set the time_till_timeout for that room to its timeout
    //     time from the config
    // ELSE:
    // - decrease the time_till_timeout for each room by 1m
    // A room is considered "in timeout" when time_till_timeout < 0

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
    interval.tick().await;
    loop {
        tokio::select! {
            // we got a temperature value in time
            read_next_result = read_next_temp_packet(&sock, &config) => {
                let time_elapsed =read_next_result.0;
                let temperatures = read_next_result.1;
                let mut lock = ext_temp.write().await;
                // update each room with the new temperature or tick town the timeout timer
                'all_rooms: for (room_name, temperature_status) in lock.iter_mut() {
                    for (name, temp) in &temperatures {
                        if name == room_name {
                            temperature_status.update_with(&name, *temp, &config)?;
                            continue 'all_rooms;
                        }
                    };
                    // no new information for this room - update its timeout status
                    temperature_status.tick_down(time_elapsed);
                };
                interval.reset();
            }
            // timeout: no correct temp value received
            _ = interval.tick() => {
                warn!("Got no external temperature within timeout. Now setting it to unknown.");
                let mut lock = ext_temp.write().await;
                // tick down each rooms timeout timer
                for (_, temperature_status) in lock.iter_mut() {
                    // no new information for this room - update its timeout status
                    temperature_status.tick_down(60);
                };
            }
            _ = watcher.changed() => {
                debug!("Shutting down the temperature receiver now.");
                return Ok(());
            }
        }
    }
}
