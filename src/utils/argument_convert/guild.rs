use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`Guild::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum GuildParseError {
    /// The provided guild string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
}

impl std::error::Error for GuildParseError {}

impl std::fmt::Display for GuildParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFoundOrMalformed => write!(f, "Guild not found or unknown format"),
        }
    }
}

/// Look up a Guild, either by ID or by a string case-insensitively.
///
/// Requires the cache feature to be enabled.
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for Guild {
    type Err = GuildParseError;

    async fn convert(
        ctx: &Context,
        _guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let guilds = ctx.cache.guilds.read().await;

        let lookup_by_id = || guilds.get(&GuildId(s.parse().ok()?));

        let lookup_by_name = || guilds.values().find(|guild| guild.name.eq_ignore_ascii_case(s));

        lookup_by_id().or_else(lookup_by_name).cloned().ok_or(GuildParseError::NotFoundOrMalformed)
    }
}
