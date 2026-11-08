use crate::easytier::{ConnectionType, EasyTier, EasyTierHolder, EasyTierMember, PortForward};
use easytier::common::config::{ConfigFileControl, ConfigLoader, TomlConfigLoader};
use easytier::launcher::NetworkInstance;
use easytier::proto::api::config::{
    ConfigPatchAction, InstanceConfigPatch, PatchConfigRequest, PortForwardPatch,
};
use easytier::proto::api::instance::{ListRouteRequest, Route, ShowNodeInfoRequest};
use easytier::proto::common::{NatType, PortForwardConfigPb};
use easytier::proto::rpc_types::controller::BaseController;
use std::future::Future;
use std::iter::once;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const RPC_TIMEOUT: Duration = Duration::from_secs(5);
const DEBUG_INTERVAL: Duration = Duration::from_secs(30);

async fn rpc_result<T, E: std::fmt::Display>(
    request: impl Future<Output = Result<T, E>>,
) -> Option<T> {
    match tokio::time::timeout(RPC_TIMEOUT, request).await {
        Ok(Ok(value)) => Some(value),
        Ok(Err(error)) => {
            logging!("EasyTier", "RPC failed: {}", error);
            None
        }
        Err(_) => {
            logging!("EasyTier", "RPC timed out.");
            None
        }
    }
}

#[cfg(target_os = "android")]
pub struct EasyTierTunRequest {
    pub address: Ipv4Addr,
    pub network_length: u8,
    pub cidrs: Vec<String>,
    pub dest: Arc<RwLock<Option<i32>>>,
}

pub fn create_with_config(config: TomlConfigLoader) -> EasyTier {
    let latency_first = config.get_flags().latency_first;
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

    let connection_type = Arc::new(RwLock::new(None));
    let connection_status = connection_type.clone();
    let connection_service = service.clone();
    // 在后台采样路由，读取大厅状态时不持有状态锁等待 RPC。
    runtime.spawn(async move {
        loop {
            let current = rpc_result(
                connection_service
                    .get_peer_manage_service()
                    .list_route(BaseController::default(), ListRouteRequest::default()),
            )
            .await
            .and_then(|routes| {
                routes
                    .routes
                    .iter()
                    .find(|route| route.hostname.starts_with(crate::rooms::SERVER_PREFIX))
                    .and_then(|route| route_connection_type(route, latency_first))
            });
            *connection_status.write().unwrap() = current;
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    let debug_output = crate::easytier::get_link_options().enable_cli_output;
    if debug_output || cfg!(target_os = "android") {
        runtime.spawn(async move {
            #[cfg(target_os = "android")]
            let mut p_address = None;
            #[cfg(target_os = "android")]
            let mut p_proxy_cidrs = vec![];
            let mut last_output = Instant::now() - DEBUG_INTERVAL;

            loop {
                let Some(node) = rpc_result(
                    service
                        .get_peer_manage_service()
                        .show_node_info(BaseController::default(), ShowNodeInfoRequest::default()),
                )
                .await
                .and_then(|my_info| my_info.node_info) else {
                    return;
                };
                let Some(routes) = rpc_result(
                    service
                        .get_peer_manage_service()
                        .list_route(BaseController::default(), ListRouteRequest::default()),
                )
                .await
                else {
                    return;
                };
                if debug_output && last_output.elapsed() >= DEBUG_INTERVAL {
                    logging!("EasyTier", "Node: {:?}; Routes: {:?}", node, routes.routes);
                    last_output = Instant::now();
                }

                #[cfg(target_os = "android")]
                {
                    let address = node
                        .ipv4_addr
                        .parse::<cidr::Ipv4Inet>()
                        .ok()
                        .map(|address| (address.address(), address.network_length()));

                    let proxy_cidrs = routes
                        .routes
                        .into_iter()
                        .flat_map(|route| route.proxy_cidrs)
                        .collect::<Vec<_>>();

                    if (p_address != address || p_proxy_cidrs != proxy_cidrs)
                        && let Some((_address, _network_length)) = address
                    {
                        crate::on_vpnservice_change(EasyTierTunRequest {
                            address: _address,
                            network_length: _network_length,
                            cidrs: proxy_cidrs.clone(),
                            dest: _tun_fd.clone(),
                        });
                    }

                    p_address = address;
                    p_proxy_cidrs = proxy_cidrs;
                }
                #[cfg(target_os = "android")]
                tokio::time::sleep(Duration::from_millis(100)).await;
                #[cfg(not(target_os = "android"))]
                tokio::time::sleep(DEBUG_INTERVAL).await;
            }
        });
    }

    EasyTier(Some(EasyTierHolder {
        instance,
        runtime,
        connection_type,
    }))
}

fn route_connection_type(route: &Route, latency_first: bool) -> Option<ConnectionType> {
    let hops = if latency_first {
        route.cost_latency_first?
    } else {
        route.cost
    };
    match hops {
        1 => Some(ConnectionType::P2p),
        2.. => Some(ConnectionType::Relay),
        _ => None,
    }
}

impl EasyTier {
    pub fn get_connection_type(&self) -> Option<ConnectionType> {
        self.0
            .as_ref()
            .and_then(|holder| *holder.connection_type.read().unwrap())
    }

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
                        runtime.block_on(async {
                            let first = rpc_result(service.list_route(
                                BaseController::default(),
                                ListRouteRequest::default(),
                            ))
                            .await?
                            .routes;

                            let second = rpc_result(service.show_node_info(
                                BaseController::default(),
                                ShowNodeInfoRequest::default(),
                            ))
                            .await?
                            .node_info?;

                            Some((first, second))
                        })
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
            let Some(service) = instance.get_api_service() else {
                return false;
            };
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

            return runtime.block_on(rpc_result(task)).is_some();
        }
        false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_type_follows_the_active_routing_policy() {
        for (hops, latency_hops, shortest, fastest) in [
            (
                1,
                Some(2),
                Some(ConnectionType::P2p),
                Some(ConnectionType::Relay),
            ),
            (
                2,
                Some(1),
                Some(ConnectionType::Relay),
                Some(ConnectionType::P2p),
            ),
            (1, None, Some(ConnectionType::P2p), None),
            (0, Some(0), None, None),
            (-1, Some(-1), None, None),
        ] {
            let route = Route {
                cost: hops,
                cost_latency_first: latency_hops,
                ..Default::default()
            };
            assert_eq!(route_connection_type(&route, false), shortest);
            assert_eq!(route_connection_type(&route, true), fastest);
        }
        assert_eq!(serde_json::to_value(ConnectionType::P2p).unwrap(), "p2p");
        assert_eq!(
            serde_json::to_value(ConnectionType::Relay).unwrap(),
            "relay"
        );
    }
}
