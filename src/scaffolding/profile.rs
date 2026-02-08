use serde::Serialize;
use crate::MACHINE_ID_FILE;
use rand_core::{OsRng, TryRngCore};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

lazy_static::lazy_static! {
    pub static ref MACHINE_ID: &'static str = get_machine_id();

    pub static ref VENDOR: &'static str = format!("Terracotta {}, EasyTier {}", env!("TERRACOTTA_VERSION"), env!("TERRACOTTA_ET_VERSION")).leak();
}

fn get_machine_id() -> &'static str {
    if let Ok(mut file) = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(MACHINE_ID_FILE.clone()) {
        let mut bytes = [0u8; 17];
        match file.read(&mut bytes) {
            Ok(16) => {},
            Ok(length) => {
                logging!("MachineID", "Cannot restore machine id: expecting 16 bytes, but {} bytes are found.", length);
                OsRng.try_fill_bytes(&mut bytes[0..16]).unwrap();
                if let Ok(_) = file.seek(SeekFrom::Start(0)) {
                    let _ = file.write(&bytes[0..16]);
                }
            },
            Err(e) => {
                logging!("MachineID", "Cannot restore machine id: {:?}", e);
            },
        }

        return hex::encode(&bytes[0..16]).leak();
    }

    let mut bytes = [0u8; 17];
    OsRng.try_fill_bytes(&mut bytes).unwrap();
    return hex::encode(&bytes).leak();
}


#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ProfileKind {
    HOST, LOCAL, GUEST
}


#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    machine_id: String,
    name: String,
    vendor: String,
    kind: ProfileKind,
}

pub struct ProfileSnapshot {
    pub machine_id: String,
    pub name: String,
    pub vendor: String,
    pub kind: ProfileKind,
}

impl ProfileSnapshot {
    pub fn into_profile(self) -> Profile {
        Profile { machine_id: self.machine_id, name: self.name, vendor: self.vendor, kind: self.kind}
    }
}

impl Profile {
    pub fn get_machine_id(&self) -> &str {
        &self.machine_id
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }

    pub fn get_vendor(&self) -> &str {
        &self.vendor
    }

    pub fn get_kind(&self) -> &ProfileKind {
        &self.kind
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}
