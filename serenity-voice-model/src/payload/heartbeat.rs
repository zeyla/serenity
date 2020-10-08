use crate::util::json_safe_u64;
use serde::{Deserialize, Serialize};

/// Used to keep the websocket connection alive.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Heartbeat {
	/// Random number generated by the client, to be mirrored by the server.
    #[serde(with = "json_safe_u64")] 
    pub nonce: u64,
}
