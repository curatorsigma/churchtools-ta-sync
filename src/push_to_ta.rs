//! Push the state from DB to CMIs

use std::sync::Arc;

use chrono::{TimeDelta, Utc};
use tokio::{net::UdpSocket, sync::RwLock};
use tracing::{debug, info, trace, warn};

use crate::{
    config::Config,
    db::{get_bookings_in_timeframe, DBError},
    InShutdown,
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

/// Send CoE packets to all cmis, updating them on the state of all their assigned rooms
async fn emit_coe(config: &Config, ext_temp: Option<i32>) -> Result<(), COEEmitError> {
    // get all bookings from the db that intersect now and now + 30 mins
    let start = Utc::now().naive_utc();
    let end = start + TimeDelta::minutes(30);
    let bookings = get_bookings_in_timeframe(&config.db, start, end).await?;

    let sock = UdpSocket::bind((config.global.cmi_bind_addr.clone(), 0)).await?;
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
                        if b.churchtools_id != room.churchtools_id {
                            return false;
                        };
                        let (new_start, new_stop) =
                            room.apply_preheat_and_preshutdown(b.start_time, b.end_time, ext_temp);
                        let now = Utc::now();
                        new_start < now && now < new_stop
                    })
                    .count();
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
        let packets = coe::packets_from_payloads(&payloads);
        // send all packets.
        for packet in packets {
            sock.send_to(&Into::<Vec<u8>>::into(packet), (cmi.host.as_str(), 5442))
                .await?;
            trace!("Sent a CoE packet to {}", cmi.host);
        }
    }
    Ok(())
}

/// Continually push data from the db to CMIs.
pub async fn push_coe(
    config: Arc<Config>,
    mut watcher: tokio::sync::watch::Receiver<InShutdown>,
    ext_temp: Arc<RwLock<Option<i32>>>,
) {
    info!("Starting DB -> TA COE emitter task");
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        config.global.ta_push_frequency * 60,
    ));
    interval.tick().await;
    loop {
        debug!("Emitter starting new run.");
        let current_temp = *ext_temp.read().await;
        // send data from state once
        let res = emit_coe(&config, current_temp).await;
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
