//! Read the external temperature from a CMI sending that information.

use std::sync::Arc;

use coe::Packet;
use tokio::{net::UdpSocket, sync::RwLock};
use tracing::{info, trace};

use crate::config::Config;

pub async fn read_next_ext_temp_packet(sock: UdpSocket, can_id: u8, pdo_index: u8) -> i32 {
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
    };
}

/// Update the external temperature whenever a corresponding value is received from a CMI.
///
/// After config.external_temperature_sensor.timeout minutes, the External Temperature is set to
/// None
pub async fn read_ext_temp(config: &Config, ext_temp: Arc<RwLock<Option<i32>>>) {
    info!("Starting external temperature receiver");
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        config.external_temperature_sensor.timeout as u64 * 60
    ));
    interval.tick().await;
    // crate UDP socket
    let sock = UdpSocket::bind((config.external_temperature_sensor.bind_addr.clone(), 5442));
    loop {
        // read the next packet
        // Try to parse it as CoE packet
        // set a global var to that value
    }
}
