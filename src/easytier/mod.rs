use crate::controller::ConnectionDifficulty;
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
use tokio::runtime::Runtime;

struct EasyTierHolder {
    instance: NetworkInstance,
    runtime: Runtime,
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

pub fn create_with_config(config: TomlConfigLoader) -> EasyTier {
    inner::create_with_config(config)
}