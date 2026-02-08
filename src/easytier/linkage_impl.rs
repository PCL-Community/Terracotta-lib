use crate::easytier::{EasyTier, EasyTierHolder, EasyTierMember, PortForward};
use easytier::common::config::{ConfigFileControl, TomlConfigLoader};
use easytier::launcher::NetworkInstance;
use easytier::proto::api::config::{
    ConfigPatchAction, InstanceConfigPatch, PatchConfigRequest, PortForwardPatch,
};
use easytier::proto::api::instance::{ListRouteRequest, ShowNodeInfoRequest};
use easytier::proto::common::{NatType, PortForwardConfigPb};
use easytier::proto::rpc_types::controller::BaseController;
use std::iter::once;
use std::net::Ipv4Addr;
use std::str::FromStr;
#[cfg(target_os = "android")]
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

#[cfg(target_os = "android")]
pub struct EasyTierTunRequest {
    pub address: Ipv4Addr,
    pub network_length: u8,
    pub cidrs: Vec<String>,
    pub dest: Arc<RwLock<Option<i32>>>,
}

pub fn create_with_config(config: TomlConfigLoader) -> EasyTier {
    let mut instance = NetworkInstance::new(config, ConfigFileControl::STATIC_CONFIG);
    let Ok((instance, runtime)) = instance
        .start()
        .map(|_| instance)
        .map_err(|e| {
            logging!("EasyTier", "Cannot launch EasyTier: {:?}", e);
        })
        .and_then(|instance| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map(|runtime| (instance, runtime))
                .map_err(|e| {
                    logging!("EasyTier", "Cannot launch Tokio: {:?}", e);
                })
        })
    else {
        return EasyTier(None);
    };
    let service = 'service: {
        thread::sleep(Duration::from_millis(1500));

        for _ in 0..20 {
            if let Some(service) = instance.get_api_service() {
                break 'service service;
            }

            thread::sleep(Duration::from_millis(500));
        }

        if let Some(notifier) = instance.get_stop_notifier() {
            notifier.notify_one();
        }
        return EasyTier(None);
    };
    let _tun_fd = instance.launcher.as_ref().unwrap().data.tun_fd.clone();

    runtime.spawn(async move {
        let mut p_address = None;
        let mut p_proxy_cidrs = vec![];

        loop {
            let address = service
                .get_peer_manage_service()
                .show_node_info(BaseController::default(), ShowNodeInfoRequest::default())
                .await
                .ok()
                .and_then(|my_info| my_info.node_info)
                .unwrap()
                .ipv4_addr
                .parse::<cidr::Ipv4Inet>()
                .ok()
                .map(|address| (address.address(), address.network_length()));

            let proxy_cidrs = service
                .get_peer_manage_service()
                .list_route(BaseController::default(), ListRouteRequest::default())
                .await
                .ok()
                .unwrap()
                .routes
                .into_iter()
                .flat_map(|route| route.proxy_cidrs)
                .collect::<Vec<_>>();

            if p_address != address || p_proxy_cidrs != proxy_cidrs {
                if let Some((_address, _network_length)) = address {
                    #[cfg(target_os = "android")]
                    crate::on_vpnservice_change(EasyTierTunRequest {
                        address: _address,
                        network_length: _network_length,
                        cidrs: proxy_cidrs.clone(),
                        dest: _tun_fd.clone(),
                    });
                    #[cfg(not(target_os = "android"))]
                    {
                        // 在非 Android 平台上，不需要处理 VPN 服务变更
                        logging!(
                            "EasyTier",
                            "VPN service change not handled on non-Android platform"
                        );
                    }
                }
            }

            p_address = address;
            p_proxy_cidrs = proxy_cidrs;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    EasyTier(Some(EasyTierHolder { instance, runtime }))
}

impl EasyTier {
    pub fn is_alive(&self) -> bool {
        self.0
            .as_ref()
            .is_some_and(|EasyTierHolder { instance, .. }| instance.is_easytier_running())
    }

    pub fn get_players(&self) -> Option<Vec<EasyTierMember>> {
        self.0
            .as_ref()
            .and_then(
                |EasyTierHolder {
                     instance, runtime, ..
                 }| {
                    instance.get_api_service().and_then(|service| {
                        let service = service.get_peer_manage_service();
                        let first =
                            runtime
                                .block_on(service.list_route(
                                    BaseController::default(),
                                    ListRouteRequest::default(),
                                ))
                                .ok()
                                .map(|response| response.routes);

                        let second = runtime
                            .block_on(service.show_node_info(
                                BaseController::default(),
                                ShowNodeInfoRequest::default(),
                            ))
                            .ok()
                            .and_then(|info| info.node_info);

                        first.and_then(|first| second.map(|second| (first, second)))
                    })
                },
            )
            .map(|(neighbours, this)| {
                fn parse_address(
                    address: Option<easytier::proto::common::Ipv4Inet>,
                ) -> Option<Ipv4Addr> {
                    address
                        .and_then(|address| address.address)
                        .map(|address| Ipv4Addr::from_octets(address.addr.to_be_bytes()))
                }
                fn parse_stun_info(
                    stun_info: Option<easytier::proto::common::StunInfo>,
                ) -> crate::easytier::NatType {
                    stun_info
                        .map(|stun| match stun.udp_nat_type() {
                            NatType::Unknown => crate::easytier::NatType::Unknown,
                            NatType::OpenInternet => crate::easytier::NatType::OpenInternet,
                            NatType::NoPat => crate::easytier::NatType::NoPAT,
                            NatType::FullCone => crate::easytier::NatType::FullCone,
                            NatType::Restricted => crate::easytier::NatType::Restricted,
                            NatType::PortRestricted => crate::easytier::NatType::PortRestricted,
                            NatType::Symmetric => crate::easytier::NatType::Symmetric,
                            NatType::SymUdpFirewall => crate::easytier::NatType::SymmetricUdpWall,
                            NatType::SymmetricEasyInc => {
                                crate::easytier::NatType::SymmetricEasyIncrease
                            }
                            NatType::SymmetricEasyDec => {
                                crate::easytier::NatType::SymmetricEasyDecrease
                            }
                        })
                        .unwrap_or(crate::easytier::NatType::Unknown)
                }

                neighbours
                    .into_iter()
                    .map(|route| EasyTierMember {
                        hostname: route.hostname,
                        address: parse_address(route.ipv4_addr),
                        nat: parse_stun_info(route.stun_info),
                        is_local: false,
                    })
                    .chain(once(EasyTierMember {
                        hostname: this.hostname,
                        address: Ipv4Addr::from_str(&this.ipv4_addr).ok(),
                        nat: parse_stun_info(this.stun_info),
                        is_local: true,
                    }))
                    .collect::<Vec<_>>()
            })
    }

    pub fn add_port_forward(&mut self, forwards: &[PortForward]) -> bool {
        if let Some(EasyTierHolder {
            instance, runtime, ..
        }) = self.0.as_ref()
        {
            let service = instance.get_api_service().unwrap();
            let task = service.get_config_service().patch_config(
                BaseController::default(),
                PatchConfigRequest {
                    patch: Some(InstanceConfigPatch {
                        port_forwards: forwards
                            .iter()
                            .map(|forward| PortForwardPatch {
                                action: ConfigPatchAction::Add as i32,
                                cfg: Some(PortForwardConfigPb {
                                    bind_addr: Some(forward.local.into()),
                                    dst_addr: Some(forward.remote.into()),
                                    socket_type: forward.socket_type.into(),
                                }),
                            })
                            .collect::<Vec<PortForwardPatch>>(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            );

            return match runtime.block_on(task) {
                Ok(_) => true,
                Err(e) => {
                    logging!("EasyTier", "Cannot adding port-forward rules: {:?}", e);
                    false
                }
            };
        }
        return false;
    }
}

impl Drop for EasyTier {
    fn drop(&mut self) {
        logging!("EasyTier", "Killing EasyTier.");

        if let Some(EasyTierHolder {
            instance, runtime, ..
        }) = self.0.take()
        {
            if let Some(msg) = instance.get_latest_error_msg() {
                logging!(
                    "EasyTier",
                    "EasyTier has encountered an fatal error: {}",
                    msg
                );
            }
            if let Some(notifier) = instance.get_stop_notifier() {
                notifier.notify_one();
            }
            runtime.shutdown_background();
        }
    }
}
