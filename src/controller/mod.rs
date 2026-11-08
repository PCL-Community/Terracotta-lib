mod api;
pub mod states;
pub use crate::easytier::{LinkOptions, ProtocolPreference};
use crate::{rooms::protocols::HANDLERS, scaffolding::server::start};
pub use api::{
    NatTestResult, get_link_options, get_meta, get_public_nodes, get_state, set_guesting,
    set_host_starting, set_link_options, set_public_nodes, set_scanning, set_scanning_only,
    set_vendor, set_waiting, test_nat,
};

lazy_static::lazy_static! {
    pub static ref SCAFFOLDING_PORT: Option<u16> = start(HANDLERS, 13448).or_else(|_| start(HANDLERS, 0)).map_err(|error| {
        logging!("ScaffoldingServer", "Cannot start listener: {:?}", error);
    }).ok();
}
