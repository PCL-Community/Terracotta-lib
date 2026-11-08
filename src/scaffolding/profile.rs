use crate::MACHINE_ID_FILE;
use rand_core::{OsRng, TryRngCore};
use serde::Serialize;
use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::RwLock;

lazy_static::lazy_static! {
    pub static ref MACHINE_ID: &'static str = get_machine_id();

    pub static ref VENDOR: &'static str = format!("Terracotta {}, EasyTier {}", env!("TERRACOTTA_VERSION"), env!("TERRACOTTA_ET_VERSION")).leak();
}

/// 宿主可注入的 vendor 前缀（例如 “PCL.Proto 0.10.0”）。
///
/// 未注入时保持历史默认值；注入后最终 vendor 为 `{base}, EasyTier {ET_VERSION}`。
/// 注入应在第一次创建/加入大厅前完成，之后的调用会覆盖此前设置。
static VENDOR_BASE: RwLock<Option<String>> = RwLock::new(None);

pub fn set_vendor(base: String) {
    if let Ok(mut guard) = VENDOR_BASE.write() {
        *guard = Some(base);
    }
}

pub fn vendor() -> String {
    match VENDOR_BASE.read().ok().and_then(|guard| guard.clone()) {
        Some(base) => format!("{base}, EasyTier {}", env!("TERRACOTTA_ET_VERSION")),
        None => (*VENDOR).to_string(),
    }
}

fn get_machine_id() -> &'static str {
    let bytes = load_machine_id(&MACHINE_ID_FILE).unwrap_or_else(|error| {
        logging!("MachineID", "Cannot persist machine id: {:?}", error);
        let mut bytes = [0u8; 16];
        OsRng
            .try_fill_bytes(&mut bytes)
            .expect("OS randomness unavailable");
        bytes
    });
    hex::encode(bytes).leak()
}

fn load_machine_id(path: &Path) -> io::Result<[u8; 16]> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    let length = file.metadata()?.len();
    let mut bytes = [0u8; 16];
    if length >= 16 {
        file.read_exact(&mut bytes)?;
    } else {
        OsRng.try_fill_bytes(&mut bytes).map_err(io::Error::other)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&bytes)?;
    }
    // Preserve the existing identity when repairing a file with trailing bytes.
    if length != 16 {
        file.set_len(16)?;
    }
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
// 这三个名字会原样进入序列化结果，改成驼峰会改变已有的配置/通信格式。
#[allow(clippy::upper_case_acronyms)]
pub enum ProfileKind {
    HOST,
    LOCAL,
    GUEST,
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
        Profile {
            machine_id: self.machine_id,
            name: self.name,
            vendor: self.vendor,
            kind: self.kind,
        }
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
