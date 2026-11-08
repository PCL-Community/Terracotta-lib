use std::cmp::PartialEq;
use std::net::{Ipv4Addr, SocketAddr};

pub mod publics;

mod linkage_impl;
use easytier::common::config::TomlConfigLoader;
use easytier::launcher::NetworkInstance;
use easytier::proto::common::SocketType;
use linkage_impl as inner;

#[cfg(target_os = "android")]
pub use inner::EasyTierTunRequest;
use std::sync::{Arc, LazyLock, RwLock};
use tokio::runtime::Runtime;

/// 传输协议偏好，映射 EasyTier 的 `default_protocol`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolPreference {
    #[default]
    Tcp,
    Udp,
}

impl ProtocolPreference {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// 会影响 EasyTier 启动参数与网络行为的联机选项。
///
/// 默认值对齐 PCL2-CE 设置页：延迟优先、端口猜测、IPv6 与 TCP 偏好均开启，
/// 调试输出关闭。选项在创建房间（含房主与加入者）时读取，修改后下一次
/// 创建/加入大厅生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LinkOptions {
    pub latency_first: bool,
    pub try_punch_sym: bool,
    pub enable_ipv6: bool,
    pub protocol: ProtocolPreference,
    pub enable_cli_output: bool,
}

impl Default for LinkOptions {
    fn default() -> Self {
        Self {
            latency_first: true,
            try_punch_sym: true,
            enable_ipv6: true,
            protocol: ProtocolPreference::Tcp,
            enable_cli_output: false,
        }
    }
}

static LINK_OPTIONS: LazyLock<RwLock<LinkOptions>> =
    LazyLock::new(|| RwLock::new(LinkOptions::default()));

/// 设置全局联机选项；设置失败（锁中毒）时静默保留旧值。
pub fn set_link_options(options: LinkOptions) {
    if let Ok(mut guard) = LINK_OPTIONS.write() {
        *guard = options;
    }
}

pub fn get_link_options() -> LinkOptions {
    LINK_OPTIONS.read().map(|guard| *guard).unwrap_or_default()
}

#[derive(Debug)]
pub enum ConnectionDifficulty {
    Unknown,
    Easiest,
    Simple,
    Medium,
    Tough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    P2p,
    Relay,
}

struct EasyTierHolder {
    instance: NetworkInstance,
    runtime: Runtime,
    connection_type: Arc<RwLock<Option<ConnectionType>>>,
}

pub struct PortForward {
    /// 本地绑定地址
    pub local: SocketAddr,
    /// 目标地址
    pub remote: SocketAddr,
    /// 使用的协议类型 tcp/udp
    pub socket_type: SocketType,
}

pub struct EasyTier(Option<EasyTierHolder>);

#[derive(Debug)]
pub struct EasyTierMember {
    pub hostname: String,
    pub address: Option<Ipv4Addr>,
    pub is_local: bool,
    pub nat: NatType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NatType {
    Unknown,
    OpenInternet,
    NoPAT,
    FullCone,
    Restricted,
    PortRestricted,
    Symmetric,
    SymmetricUdpWall,
    SymmetricEasyIncrease,
    SymmetricEasyDecrease,
}

pub fn calc_conn_difficulty(left: &NatType, right: &NatType) -> ConnectionDifficulty {
    let is = |types: &[NatType]| -> bool { types.contains(left) || types.contains(right) };

    if is(&[NatType::OpenInternet]) {
        ConnectionDifficulty::Easiest
    } else if is(&[NatType::NoPAT, NatType::FullCone]) {
        ConnectionDifficulty::Simple
    } else if is(&[NatType::Restricted, NatType::PortRestricted]) {
        ConnectionDifficulty::Medium
    } else {
        ConnectionDifficulty::Tough
    }
}

#[inline]
pub fn create_with_config(config: TomlConfigLoader) -> EasyTier {
    inner::create_with_config(config)
}
