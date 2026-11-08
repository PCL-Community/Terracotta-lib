use crate::controller::states::{AppState, ExceptionType};
use crate::easytier::ConnectionDifficulty;
use crate::easytier::publics::fetch_public_nodes;
use crate::mc::fakeserver::is_lobby_motd;
use crate::mc::scanning::MinecraftScanner;
use crate::rooms::Room;
use crate::scaffolding::profile::{Profile, set_vendor as profile_set_vendor, vendor};
use rocket::serde::Serialize;
use serde::Serializer;
use serde::ser::SerializeSeq;
use serde_json::{Value, json};
use std::sync::mpsc;
use std::sync::{LazyLock, RwLock};
use std::thread;
use std::time::{Duration, SystemTime};

/// 宿主注入的公共节点（EasyTier peer），由公告/设置页决定。
///
/// 这些节点会在每次创建/加入大厅时与 `publics::fetch_public_nodes` 的内置
/// 节点合并后作为 EasyTier peers 使用。
static PUBLIC_NODES: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(Vec::new()));

const NAT_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// NAT 网络测试结果。
///
/// `udp_nat_type` / `tcp_nat_type` 使用 PCL2-CE `CliNetTest.GetNatTypeViaCode`
/// 的 0..8 码表：0 = OpenInternet、1 = NoPat、2 = FullCone、3 = Restricted、
/// 4 = PortRestricted、5 = SymmetricEasy、6 = Symmetric、7 = SymmetricFirewall、
/// 8 = UdpBlocked；-1 表示 Unknown（EasyTier 未给出结果或出现未知枚举）。
///
/// EasyTier proto 的 `NatType` 是 0 起的另一套编号（0 = Unknown、1 = OpenInternet、
/// …），这里已转换为 CE 文案码表，宿主可直接查 `GetNatTypeViaCode` 对应的映射。
#[derive(Debug, Clone, Serialize)]
pub struct NatTestResult {
    pub udp_nat_type: i32,
    pub tcp_nat_type: i32,
    pub public_ips: Vec<String>,
    pub ipv6: bool,
}

pub fn get_state() -> Value {
    let state = AppState::acquire();
    let (index, sharing_index) = state.index();

    match state.as_ref() {
        AppState::Waiting => {
            json!({"state": "waiting", "index": index})
        }

        AppState::HostScanning { scanner, .. } => {
            json!({"state": "host-scanning", "index": index, "avaliable_mc_ports": scanner.get_ports()})
        }
        AppState::HostStarting { room, .. } => {
            json!({"state": "host-starting", "index": index, "room": room.code})
        }
        AppState::HostOk { room, profiles, .. } => {
            struct Holder<'a>(&'a Vec<(SystemTime, Profile)>);
            impl<'a> Serialize for Holder<'a> {
                fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
                    for (_, profile) in self.0 {
                        sequence.serialize_element(profile)?;
                    }
                    sequence.end()
                }
            }

            json!({"state": "host-ok", "index": index, "room": room.code, "profile_index": sharing_index, "profiles": Holder(profiles)})
        }

        AppState::GuestConnecting { room, .. } => {
            json!({"state": "guest-connecting", "index": index, "room": room.code})
        }
        AppState::GuestStarting {
            room, difficulty, ..
        } => {
            json!({"state": "guest-starting", "index": index, "room": room.code, "difficulty": match difficulty {
                ConnectionDifficulty::Unknown => "UNKNOWN",
                ConnectionDifficulty::Easiest => "EASIEST",
                ConnectionDifficulty::Simple => "SIMPLE",
                ConnectionDifficulty::Medium => "MEDIUM",
                ConnectionDifficulty::Tough => "TOUGH",
            }})
        }
        AppState::GuestOk {
            room,
            server,
            profiles,
            latency,
            connection_type,
            ..
        } => {
            let url = if server.port == 25565 {
                "127.0.0.1".into()
            } else {
                format!("127.0.0.1:{}", server.port)
            };

            json!({"state": "guest-ok", "index": index, "room": room.code, "url": url, "latency": latency, "connection_type": connection_type, "profile_index": sharing_index, "profiles": profiles})
        }
        AppState::Exception { kind, .. } => json!({
            "state": "exception",
            "index": index,
            "type": match kind {
                ExceptionType::PingHostFail => 0,
                ExceptionType::PingHostRst => 1,
                ExceptionType::GuestEasytierCrash => 2,
                ExceptionType::HostEasytierCrash => 3,
                ExceptionType::PingServerRst => 4,
                ExceptionType::ScaffoldingInvalidResponse => 5,
            }
        }),
    }
}

pub fn set_waiting() {
    logging!("Core", "Setting to state WAITING.");

    let state = AppState::acquire().into_slow();
    if matches!(state.as_ref(), AppState::Waiting) {
        return;
    }
    state.set(AppState::Waiting);
}

/// 覆盖全局公共节点列表；下一次创建/加入大厅时生效。
pub fn set_public_nodes(nodes: Vec<String>) {
    if let Ok(mut guard) = PUBLIC_NODES.write() {
        *guard = nodes;
    }
}

pub fn get_public_nodes() -> Vec<String> {
    PUBLIC_NODES
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// 合并宿主注入的节点与调用参数，再补齐内置公开节点。
fn resolve_public_nodes(extra: Vec<String>) -> Vec<String> {
    let mut nodes = get_public_nodes();
    nodes.extend(extra);
    fetch_public_nodes(nodes)
}

pub fn set_scanning(room: Option<String>, player: Option<String>, public_nodes: Vec<String>) {
    let capture = {
        let state = AppState::acquire();
        if !matches!(state.as_ref(), AppState::Waiting) {
            return;
        }

        state.set(AppState::HostScanning {
            scanner: MinecraftScanner::create(|motd| !is_lobby_motd(motd)),
        })
    };
    logging!("Core", "Setting to state SCANNING.");

    thread::spawn(move || {
        let room = room
            .and_then(|room| Room::from(&room))
            .unwrap_or_else(Room::create);

        let (room, port, capture) = loop {
            thread::sleep(Duration::from_millis(200));

            let Some(state) = capture.try_capture() else {
                return;
            };
            let AppState::HostScanning { scanner, .. } = state.as_ref() else {
                unreachable!()
            };

            if let Some(port) = scanner.get_ports().first() {
                break (
                    room.clone(),
                    *port,
                    state.set(AppState::HostStarting { room, port: *port }),
                );
            }
        };

        room.start_host(port, player, capture, &resolve_public_nodes(public_nodes))
    });
}

pub fn set_scanning_only() {
    let state = AppState::acquire();
    if !matches!(state.as_ref(), AppState::Waiting) {
        return;
    }
    state.set(AppState::HostScanning {
        scanner: MinecraftScanner::create(|motd| !is_lobby_motd(motd)),
    });
    logging!("Core", "Setting to state SCANNING.");
}

/// 房主开始联机。允许从 `HostScanning`（由扫描线程选定端口）或 `Waiting`
/// （宿主直接指定端口，跳过自动扫描）进入。
pub fn set_host_starting(mc_port: u16, player: Option<String>) -> Option<String> {
    let state = AppState::acquire();
    if !matches!(
        state.as_ref(),
        AppState::HostScanning { .. } | AppState::Waiting
    ) {
        return None;
    }
    let room = Room::create();
    let room_code = room.code.clone();
    logging!("Core", "Created room, code={}", &room_code);
    let capture = state.set(AppState::HostStarting {
        room: room.clone(),
        port: mc_port,
    });
    logging!("Core", "Setting to state HOST_STARTING.");
    thread::spawn(move || {
        room.start_host(mc_port, player, capture, &resolve_public_nodes(Vec::new()))
    });
    Some(room_code)
}

pub fn set_guesting(room: Room, player: Option<String>, public_nodes: Vec<String>) -> bool {
    let capture = {
        let state = AppState::acquire();
        if !matches!(state.as_ref(), AppState::Waiting) {
            return false;
        }
        state.set(AppState::GuestConnecting { room: room.clone() })
    };
    logging!("Core", "Connecting to room, code={}", room.code);
    thread::spawn(move || {
        let public_nodes = resolve_public_nodes(public_nodes);
        room.start_guest(player, capture, &public_nodes);
    });

    true
}

/// 覆盖全局 EasyTier 联机选项；下一次创建/加入大厅时生效。
pub fn set_link_options(options: crate::easytier::LinkOptions) {
    crate::easytier::set_link_options(options);
}

pub fn get_link_options() -> crate::easytier::LinkOptions {
    crate::easytier::get_link_options()
}

/// 覆盖玩家列表里展示的 vendor 前缀；最终值见 `profile::vendor`。
pub fn set_vendor(base: String) {
    profile_set_vendor(base);
}

/// 返回启动器展示所需的联机元信息。
pub fn get_meta() -> Value {
    json!({
        "version": env!("TERRACOTTA_VERSION"),
        "easyTierVersion": env!("TERRACOTTA_ET_VERSION"),
        "vendor": vendor(),
        "motd": crate::MOTD,
    })
}

/// 执行一次 NAT 类型网络测试。
///
/// 通过 EasyTier 的默认 STUN 服务器采集 UDP/TCP NAT 类型与公网地址，
/// 最多等待 10 秒；超时或采集失败返回 `None`。实现放在独立线程里自建
/// current-thread runtime，避免在调用方可能存在的 Tokio runtime 中嵌套。
pub fn test_nat() -> Option<NatTestResult> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(run_nat_test());
    });
    receiver
        .recv_timeout(NAT_TEST_TIMEOUT + Duration::from_millis(500))
        .ok()
        .flatten()
}

/// 把 EasyTier proto 的 `NatType` 映射成 PCL2-CE 的 0..8 码表。
fn ce_nat_type_code(nat_type: i32) -> i32 {
    use easytier::proto::common::NatType;

    match nat_type {
        x if x == NatType::OpenInternet as i32 => 0,
        x if x == NatType::NoPat as i32 => 1,
        x if x == NatType::FullCone as i32 => 2,
        x if x == NatType::Restricted as i32 => 3,
        x if x == NatType::PortRestricted as i32 => 4,
        x if x == NatType::SymmetricEasyInc as i32 => 5,
        x if x == NatType::SymmetricEasyDec as i32 => 5,
        x if x == NatType::Symmetric as i32 => 6,
        x if x == NatType::SymUdpFirewall as i32 => 7,
        _ => -1,
    }
}

fn run_nat_test() -> Option<NatTestResult> {
    use easytier::common::stun::{StunInfoCollector, StunInfoCollectorTrait as _};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let result = runtime.block_on(async {
        let collector = StunInfoCollector::new_with_default_servers();
        collect_nat_result(|| collector.get_stun_info(), NAT_TEST_TIMEOUT).await
    });
    runtime.shutdown_timeout(Duration::from_millis(500));
    result
}

async fn collect_nat_result(
    get_info: impl Fn() -> easytier::proto::common::StunInfo,
    window: Duration,
) -> Option<NatTestResult> {
    use easytier::proto::common::NatType;

    get_info(); // Starts the independent UDP, TCP and IPv6 collectors.
    tokio::time::sleep(window).await;
    let info = get_info();
    if info.udp_nat_type == NatType::Unknown as i32
        && info.tcp_nat_type == NatType::Unknown as i32
        && info.public_ip.is_empty()
    {
        return None;
    }
    Some(NatTestResult {
        udp_nat_type: ce_nat_type_code(info.udp_nat_type),
        tcp_nat_type: ce_nat_type_code(info.tcp_nat_type),
        ipv6: info.public_ip.iter().any(|ip| ip.contains(':')),
        public_ips: info.public_ip,
    })
}
