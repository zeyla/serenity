use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`Channel::convert`].
#[non_exhaustive]
#[derive(Debug)]
pub enum ChannelParseError {
    /// When channel retrieval via HTTP failed
    Http(SerenityError),
    /// The provided channel string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
}

impl std::error::Error for ChannelParseError {}

impl std::fmt::Display for ChannelParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "Failed to request channel via HTTP: {}", e),
            Self::NotFoundOrMalformed => write!(f, "Channel not found or unknown format"),
        }
    }
}

fn channel_belongs_to_guild(channel: &Channel, guild: GuildId) -> bool {
    match channel {
        Channel::Guild(channel) => channel.guild_id == guild,
        Channel::Category(channel) => channel.guild_id == guild,
        Channel::Private(_channel) => false,
    }
}

async fn lookup_channel_global(ctx: &Context, s: &str) -> Result<Channel, ChannelParseError> {
    if let Some(channel_id) = s.parse::<u64>().ok().or_else(|| crate::utils::parse_channel(s)) {
        return ChannelId(channel_id).to_channel(ctx).await.map_err(ChannelParseError::Http);
    }

    let channels = ctx.cache.channels.read().await;
    if let Some(channel) =
        channels.values().find(|channel| channel.name.eq_ignore_ascii_case(s)).cloned()
    {
        return Ok(Channel::Guild(channel));
    }

    Err(ChannelParseError::NotFoundOrMalformed)
}

/// Look up a Channel by a string case-insensitively.
///
/// Lookup are done via local guild. If in DMs, the global cache is used instead.
///
/// The cache feature needs to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by ID.
/// 2. [Lookup by mention](`crate::utils::parse_channel`).
/// 3. Lookup by name.
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for Channel {
    type Err = ChannelParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let channel = lookup_channel_global(ctx, s).await?;

        // Don't yield for other guilds' channels
        if let Some(guild_id) = guild_id {
            if !channel_belongs_to_guild(&channel, guild_id) {
                return Err(ChannelParseError::NotFoundOrMalformed);
            }
        };

        Ok(channel)
    }
}

/// Error that can be returned from [`GuildChannel::convert`].
#[non_exhaustive]
#[derive(Debug)]
pub enum GuildChannelParseError {
    /// When channel retrieval via HTTP failed
    Http(SerenityError),
    /// The provided channel string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
    /// When the referenced channel is not a guild channel
    NotAGuildChannel,
}

impl std::error::Error for GuildChannelParseError {}

impl std::fmt::Display for GuildChannelParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "Failed to request channel via HTTP: {}", e),
            Self::NotFoundOrMalformed => write!(f, "Channel not found or unknown format"),
            Self::NotAGuildChannel => write!(f, "Channel is not a guild channel"),
        }
    }
}

/// Look up a GuildChannel by a string case-insensitively.
///
/// Lookup is done by the global cache, hence the cache feature needs to be enabled.
///
/// For more information, see the ArgumentConvert implementation for [`Channel`]
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for GuildChannel {
    type Err = GuildChannelParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        match Channel::convert(ctx, guild_id, channel_id, s).await {
            Ok(Channel::Guild(channel)) => Ok(channel),
            Ok(_) => Err(GuildChannelParseError::NotAGuildChannel),
            Err(ChannelParseError::Http(e)) => Err(GuildChannelParseError::Http(e)),
            Err(ChannelParseError::NotFoundOrMalformed) => {
                Err(GuildChannelParseError::NotFoundOrMalformed)
            },
        }
    }
}

/// Error that can be returned from [`ChannelCategory::convert`].
#[non_exhaustive]
#[derive(Debug)]
pub enum ChannelCategoryParseError {
    /// When channel retrieval via HTTP failed
    Http(SerenityError),
    /// The provided channel string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
    /// When the referenced channel is not a channel category
    NotAChannelCategory,
}

impl std::error::Error for ChannelCategoryParseError {}

impl std::fmt::Display for ChannelCategoryParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "Failed to request channel via HTTP: {}", e),
            Self::NotFoundOrMalformed => write!(f, "Channel not found or unknown format"),
            Self::NotAChannelCategory => write!(f, "Channel is not a channel category"),
        }
    }
}

/// Look up a ChannelCategory by a string case-insensitively.
///
/// Lookup is done by the global cache, hence the cache feature needs to be enabled.
///
/// For more information, see the ArgumentConvert implementation for [`Channel`]
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for ChannelCategory {
    type Err = ChannelCategoryParseError;

    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        match Channel::convert(ctx, guild_id, channel_id, s).await {
            Ok(Channel::Category(channel)) => Ok(channel),
            // TODO: accomodate issue #1352 somehow
            Ok(_) => Err(ChannelCategoryParseError::NotAChannelCategory),
            Err(ChannelParseError::Http(e)) => Err(ChannelCategoryParseError::Http(e)),
            Err(ChannelParseError::NotFoundOrMalformed) => {
                Err(ChannelCategoryParseError::NotFoundOrMalformed)
            },
        }
    }
}
