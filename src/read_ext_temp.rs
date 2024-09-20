//! Read the external temperature from a CMI sending that information.

use std::sync::Arc;

use coe::{AnalogueCOEValue, COEValue, Packet};
use tokio::{net::UdpSocket, sync::RwLock};
use tracing::{debug, error, info, trace, warn};

use crate::{config::Config, InShutdown};

pub async fn read_next_ext_temp_packet(sock: &UdpSocket, can_id: u8, pdo_index: u8) -> i32 {
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
                        for payload in packet.iter() {
                            if payload.node() == can_id && payload.pdo_index() == pdo_index {
                                if let COEValue::Analogue(
                                    AnalogueCOEValue::DegreeCentigrade_Tens(x),
                                ) = payload.value()
                                {
                                    debug!("Got the external temperature: {} °C", x as f32 / 10_f32);
                                    return x;
                                } else {
                                    trace!("Got Payload for correct ID and Index, but the Unit was not Degree Centigrade ({}).", payload.unit_id());
                                }
                            } else {
                                debug!("Got a well-formed COE packet, but it was for the wrong CAN-ID or pdo_index.");
                            }
                        }
                    }
                    Err(e) => {
                        trace!("Packet received, but not parsable as CoE: {e}");
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
}
impl std::fmt::Display for ReadExtTempError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Udp(x) => write!(f, "Udp Error: {x}"),
        }
    }
}
impl From<std::io::Error> for ReadExtTempError {
    fn from(value: std::io::Error) -> Self {
        Self::Udp(value)
    }
}
impl std::error::Error for ReadExtTempError {}

/// Update the external temperature whenever a corresponding value is received from a CMI.
///
/// After config.external_temperature_sensor.timeout minutes, the External Temperature is set to
/// None
pub async fn read_ext_temp(
    config: Arc<Config>,
    ext_temp: Arc<RwLock<Option<i32>>>,
    mut watcher: tokio::sync::watch::Receiver<InShutdown>,
    shutdown_tx: tokio::sync::watch::Sender<InShutdown>,
) -> Result<(), ReadExtTempError> {
    info!("Starting external temperature receiver");
    // crate Udp socket
    let sock =
        match UdpSocket::bind((config.external_temperature_sensor.bind_addr.clone(), 5442)).await {
            Ok(x) => x,
            Err(e) => {
                error!("Unable to open Udp Socket to listen for incoming external temperature.");
                shutdown_tx.send_replace(InShutdown::Yes);
                return Err(e.into());
            }
        };

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        config.external_temperature_sensor.timeout as u64 * 60,
    ));
    interval.tick().await;
    loop {
        tokio::select! {
            // we got a temperature value in time
            temp = read_next_ext_temp_packet(&sock, config.external_temperature_sensor.can_id, config.external_temperature_sensor.pdo_index) => {
                {
                    let mut lock = ext_temp.write().await;
                    *lock = Some(temp);
                }
                interval.reset();
            }
            // timeout: no correct temp value received
            _ = interval.tick() => {
                warn!("Got no external temperature within timeout. Now setting it to unknown.");
                let mut lock = ext_temp.write().await;
                *lock = None;
            }
            _ = watcher.changed() => {
                debug!("Shutting down the temperature receiver now");
                return Ok(());
            }
        }
    }
}
