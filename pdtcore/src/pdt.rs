use std::net::IpAddr;

use chrono::prelude::*;
use ulid::Ulid;

/// Personal Device Telemetry client info
pub struct ClientInfo {
    pub id: Ulid,
    pub name: String,
    pub ip: IpAddr,
    pub up_since: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl ClientInfo {
    pub fn new(name: String, ip: std::net::IpAddr) -> Self {
        Self {
            id: Ulid::new(),
            name,
            ip,
            up_since: Utc::now(),
            last_seen: Utc::now(),
        }
    }

    pub fn update_seen(&mut self) {
        self.last_seen = Utc::now();
    }
}

/// Personal Device Telemetry server info
pub struct ServerInfo {
    pub name: String,
    pub up_since: DateTime<Utc>,
}

impl ServerInfo {
    fn new(name: String) -> Self {
        Self {
            name,
            up_since: Utc::now(),
        }
    }
}
