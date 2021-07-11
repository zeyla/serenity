use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`Emoji::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum EmojiParseError {
    /// The provided emoji string failed to parse, or the parsed result cannot be found in the
    /// cache.
    NotFoundOrMalformed,
}

impl std::error::Error for EmojiParseError {}

impl std::fmt::Display for EmojiParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFoundOrMalformed => write!(f, "Emoji not found or unknown format"),
        }
    }
}

/// Look up a [`Emoji`].
///
/// Requires the cache feature to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by ID.
/// 2. [Lookup by extracting ID from the emoji](`crate::utils::parse_emoji`).
/// 3. Lookup by name.
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for Emoji {
    type Err = EmojiParseError;

    async fn convert(
        ctx: &Context,
        _guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let guilds = ctx.cache.guilds.read().await;

        let direct_id = s.parse::<u64>().ok().map(EmojiId);
        let id_from_mention = crate::utils::parse_emoji(s).map(|e| e.id);

        if let Some(emoji_id) = direct_id.or(id_from_mention) {
            if let Some(emoji) = guilds.values().find_map(|guild| guild.emojis.get(&emoji_id)) {
                return Ok(emoji.clone());
            }
        }

        if let Some(emoji) = guilds
            .values()
            .flat_map(|guild| guild.emojis.values())
            .find(|emoji| emoji.name.eq_ignore_ascii_case(s))
        {
            return Ok(emoji.clone());
        }

        Err(EmojiParseError::NotFoundOrMalformed)
    }
}
