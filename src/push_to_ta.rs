//! Push the state from DB to CMIs

use std::{collections::HashMap, sync::Arc};

use chrono::{TimeDelta, Utc};
use coe::Payload;
use tokio::{net::UdpSocket, sync::RwLock};
use tracing::{debug, info, trace, warn};

use crate::{
    config::Config,
    db::{get_bookings_in_timeframe, DBError},
    read_ext_temp::RoomTemperatureStatus,
    Booking, InShutdown,
};

/// All the things that can go wrong while emiting COE Packets
pub enum COEEmitError {
    /// Getting data from the DB failed
    Db(DBError),
    /// Sending packets via Udp failed
    Udp(std::io::Error),
}
impl From<DBError> for COEEmitError {
    fn from(value: DBError) -> Self {
        Self::Db(value)
    }
}
impl From<std::io::Error> for COEEmitError {
    fn from(value: std::io::Error) -> Self {
        Self::Udp(value)
    }
}
impl std::fmt::Display for COEEmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Db(e) => write!(f, "DBError: {e}"),
            Self::Udp(e) => write!(f, "Udp Error: {e}"),
        }
    }
}

fn get_packets_to_emit(
    config: &Config,
    bookings: Vec<Booking>,
    ext_temp: tokio::sync::RwLockReadGuard<HashMap<String, RoomTemperatureStatus>>,
) -> Vec<(String, Vec<Payload>)> {
    let mut all_payloads = Vec::<(String, Vec<Payload>)>::new();
    // for each CMI: send either on or off for the rooms we care about
    for cmi in &config.cmis {
        // calculate their preheating-times and cooldown-times
        //  use this to filter out the really relevant ones
        let payloads = cmi
            .rooms
            .iter()
            .map(|room| {
                let num_of_bookings_in_room = bookings
                    .iter()
                    .filter(|&b| {
                        if b.resource_id != room.room_config.churchtools_id {
                            return false;
                        };
                        if let Some(current_temp) = ext_temp.get(&room.name) {
                            return room.heat_now(
                                current_temp,
                                config
                                    .current_temperature
                                    .global_assume_current_temperature_offset,
                                b,
                            );
                        } else {
                            return false;
                        };
                    })
                    .count();
                if num_of_bookings_in_room != 0 {
                    info!("Now sending HEATING status for room {}.", room.name);
                };
                // only heat, if Utc::now() is between
                coe::Payload::new(
                    cmi.our_virtual_can_id,
                    room.pdo_index,
                    // heat the room, if at least one booking is currently in the room
                    coe::COEValue::Digital(coe::DigitalCOEValue::OnOff(
                        num_of_bookings_in_room >= 1,
                    )),
                )
            })
            .collect::<Vec<_>>();
        all_payloads.push((cmi.host.clone(), payloads));
    }
    all_payloads
}

/// Send CoE packets to all cmis, updating them on the state of all their assigned rooms
async fn emit_coe(
    config: &Config,
    ext_temp: Arc<RwLock<HashMap<String, RoomTemperatureStatus>>>,
) -> Result<(), COEEmitError> {
    // get all bookings from the db that intersect now and now + 1d
    let start = Utc::now().naive_utc();
    let end = start + TimeDelta::days(1);
    let bookings = get_bookings_in_timeframe(&config.db, start, end).await?;

    let sock = UdpSocket::bind((config.global.emiter_bind_addr.clone(), 0)).await?;

    let payloads_by_target = {
        let temperatures_unlocked = ext_temp.read().await;
        get_packets_to_emit(config, bookings, temperatures_unlocked)
    };

    // send all packets.
    for (target_host, payloads) in payloads_by_target {
        let packets = coe::packets_from_payloads(&payloads);
        for packet in packets {
            sock.send_to(&Into::<Vec<u8>>::into(packet), (target_host.as_str(), 5442))
                .await?;
            trace!("Sent a CoE packet to {}", target_host);
        }
    }
    Ok(())
}

/// Continually push data from the db to CMIs.
pub async fn push_coe(
    config: Arc<Config>,
    mut watcher: tokio::sync::watch::Receiver<InShutdown>,
    ext_temp: Arc<RwLock<HashMap<String, RoomTemperatureStatus>>>,
) {
    info!("Starting DB -> TA COE emitter task");
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        config.global.ta_push_frequency * 60,
    ));
    interval.tick().await;
    loop {
        debug!("Emitter starting new run.");
        // send data from state once
        let res = emit_coe(&config, ext_temp.clone()).await;
        match res {
            Ok(()) => {
                debug!("Successfully emitted all required CoE packets");
            }
            Err(e) => {
                warn!("An Error occured while emitting CoE packets: {e}");
            }
        }
        // stop on cancellation or continue after the next tick
        tokio::select! {
            _ = watcher.changed() => {
                debug!("Shutting down data emiter now.");
                return;
            }
            _ = interval.tick() => {}
        }
    }
}
