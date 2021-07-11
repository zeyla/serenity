use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`Role::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum RoleParseError {
    /// When the operation was invoked outside a guild.
    NotInGuild,
    /// When the guild's roles were not found in cache.
    NotInCache,
    /// The provided channel string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
}

impl std::error::Error for RoleParseError {}

impl std::fmt::Display for RoleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotInGuild => f.write_str("Must invoke this operation in a guild"),
            Self::NotInCache => f.write_str("Guild's roles were not found in cache"),
            Self::NotFoundOrMalformed => f.write_str("Role not found or unknown format"),
        }
    }
}

/// Look up a [`Role`] by a string case-insensitively.
///
/// Requires the cache feature to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by ID
/// 2. [Lookup by mention](`crate::utils::parse_role`).
/// 3. Lookup by name (case-insensitive)
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for Role {
    type Err = RoleParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let roles = ctx
            .cache
            .guild_roles(guild_id.ok_or(RoleParseError::NotInGuild)?)
            .await
            .ok_or(RoleParseError::NotInCache)?;

        if let Some(role_id) = s.parse::<u64>().ok().or_else(|| crate::utils::parse_role(s)) {
            if let Some(role) = roles.get(&RoleId(role_id)) {
                return Ok(role.clone());
            }
        }

        if let Some(role) = roles.values().find(|role| role.name.eq_ignore_ascii_case(s)) {
            return Ok(role.clone());
        }

        Err(RoleParseError::NotFoundOrMalformed)
    }
}
