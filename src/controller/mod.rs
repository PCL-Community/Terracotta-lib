mod api;
pub mod states;
use crate::{rooms::protocols::HANDLERS, scaffolding::server::start};
pub use api::{
    get_state, set_guesting, set_host_starting, set_scanning, set_scanning_only, set_waiting,
};

lazy_static::lazy_static! {
    pub static ref SCAFFOLDING_PORT: u16 = start(HANDLERS, 13448).unwrap_or_else(|_| start(HANDLERS, 0).unwrap());
}
