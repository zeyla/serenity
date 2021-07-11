use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`PLACEHOLDER::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum PLACEHOLDERParseError {
}

impl std::error::Error for PLACEHOLDERParseError {}

impl std::fmt::Display for PLACEHOLDERParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
        }
    }
}

/// Look up a [`PLACEHOLDER`] by a string case-insensitively.
///
/// Requires the cache feature to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by PLACEHOLDER
/// 2. [Lookup by PLACEHOLDER](`crate::utils::parse_PLACEHOLDER`).
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for PLACEHOLDER {
    type Err = PLACEHOLDERParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let lookup_by_PLACEHOLDER = || PLACEHOLDER;

        lookup_by_PLACEHOLDER()
            .or_else(lookup_by_PLACEHOLDER)
            .or_else(lookup_by_PLACEHOLDER)
            .cloned()
            .ok_or(PLACEHOLDERParseError::NotFoundOrMalformed)
    }
}
