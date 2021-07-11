use super::ArgumentConvert;
use crate::{model::prelude::*, prelude::*};

/// Error that can be returned from [`User::convert`].
#[non_exhaustive]
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum UserParseError {
    /// The provided user string failed to parse, or the parsed result cannot be found in the
    /// guild cache data.
    NotFoundOrMalformed,
}

impl std::error::Error for UserParseError {}

impl std::fmt::Display for UserParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFoundOrMalformed => write!(f, "User not found or unknown format"),
        }
    }
}

/// Look up a user by a string case-insensitively.
///
/// Requires the cache feature to be enabled.
///
/// The lookup strategy is as follows (in order):
/// 1. Lookup by ID.
/// 2. [Lookup by mention](`crate::utils::parse_username`).
/// 3. [Lookup by name#discrim](`crate::utils::parse_user_tag`).
/// 4. Lookup by name
#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl ArgumentConvert for User {
    type Err = UserParseError;

    async fn convert(
        ctx: &Context,
        _guild_id: Option<GuildId>,
        _channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        let users = ctx.cache.users.read().await;

        let lookup_by_id = || users.get(&UserId(s.parse().ok()?));

        let lookup_by_mention = || users.get(&UserId(crate::utils::parse_username(s)?));

        let lookup_by_name_and_discrim = || {
            let (name, discrim) = crate::utils::parse_user_tag(s)?;
            users
                .values()
                .find(|user| user.discriminator == discrim && user.name.eq_ignore_ascii_case(name))
        };

        let lookup_by_name = || users.values().find(|user| user.name == s);

        lookup_by_id()
            .or_else(lookup_by_mention)
            .or_else(lookup_by_name_and_discrim)
            .or_else(lookup_by_name)
            .cloned()
            .ok_or(UserParseError::NotFoundOrMalformed)
    }
}
