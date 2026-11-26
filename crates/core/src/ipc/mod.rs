//! Контракт локального JSON-RPC между демоном и клиентами.

mod auth;
pub mod client;
mod messages;

pub use auth::authorize_peer;
pub use messages::*;
