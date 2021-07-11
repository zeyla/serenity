use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`Member::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum MemberParseError {
    /// Parser was invoked outside a guild.
    OutsideGuild,
    /// The guild in which the parser was invoked is not in cache.
    GuildNotInCache,
    /// The provided member string failed to parse, or the parsed result cannot be found in the
    /// guild cache data.
    NotFoundOrMalformed,
}

impl std::error::Error for MemberParseError {}

impl std::fmt::Display for MemberParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideGuild => write!(f, "Tried to find member outside a guild"),
            Self::GuildNotInCache => write!(f, "Guild is not in cache"),
            Self::NotFoundOrMalformed => write!(f, "Member not found or unknown format"),
        }
    }
}

/// Look up a guild member by a string case-insensitively.
///
/// Requires the cache feature to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by ID.
/// 2. [Lookup by mention](`crate::utils::parse_username`).
/// 3. [Lookup by name#discrim](`crate::utils::parse_user_tag`).
/// 4. Lookup by name
/// 5. Lookup by nickname
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for Member {
    type Err = MemberParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let guild = guild_id
            .ok_or(MemberParseError::OutsideGuild)?
            .to_guild_cached(ctx)
            .await
            .ok_or(MemberParseError::GuildNotInCache)?;

        // DON'T use guild.members: it's only populated when guild presences intent is enabled!

        // If string is a raw user ID or a mention
        if let Some(user_id) = s.parse().ok().or_else(|| crate::utils::parse_username(s)) {
            if let Ok(member) = guild.member(ctx, UserId(user_id)).await {
                return Ok(member);
            }
        }

        // Following code is inspired by discord.py's MemberConvert::query_member_named

        // If string is a username+discriminator
        if let Some((name, discrim)) = crate::utils::parse_user_tag(s) {
            if let Ok(member_results) = guild.search_members(ctx, name, Some(100)).await {
                if let Some(member) = member_results.into_iter().find(|m| {
                    m.user.name.eq_ignore_ascii_case(name) && m.user.discriminator == discrim
                }) {
                    return Ok(member);
                }
            }
        }

        // If string is username or nickname
        if let Ok(member_results) = guild.search_members(ctx, s, Some(100)).await {
            if let Some(member) = member_results.into_iter().find(|m| {
                m.user.name.eq_ignore_ascii_case(s)
                    || m.nick.as_ref().map_or(false, |nick| nick.eq_ignore_ascii_case(s))
            }) {
                return Ok(member);
            }
        }

        Err(MemberParseError::NotFoundOrMalformed)
    }
}
