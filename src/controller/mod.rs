mod api;
pub mod states;

pub use api::{get_state, set_waiting, set_scanning, set_guesting};

use crate::{rooms::protocols::HANDLERS, scaffolding::server::start};

lazy_static::lazy_static! {
    pub static ref SCAFFOLDING_PORT: u16 = start(HANDLERS, 13448).unwrap_or_else(|_| start(HANDLERS, 0).unwrap());
}
