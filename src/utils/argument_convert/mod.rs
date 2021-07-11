mod member;
pub use member::*;

mod message;
pub use message::*;

mod user;
pub use user::*;

mod channel;
pub use channel::*;

mod guild;
pub use guild::*;

mod role;
pub use role::*;

mod emoji;
pub use emoji::*;

use crate::model::prelude::*;
use crate::prelude::*;

#[deprecated(note = "Superseded by ArgumentConvert trait")]
#[async_trait::async_trait]
pub trait Parse: Sized {
    /// The associated error which can be returned from parsing.
    type Err;

    /// Parses a string `s` as a command parameter of this type.
    async fn parse(ctx: &Context, msg: &Message, s: &str) -> Result<Self, Self::Err>;
}

#[allow(deprecated)]
#[async_trait::async_trait]
impl<T: ArgumentConvert> Parse for T {
    type Err = <T as ArgumentConvert>::Err;

    async fn parse(ctx: &Context, msg: &Message, s: &str) -> Result<Self, Self::Err> {
        Self::convert(ctx, msg.guild_id, Some(msg.channel_id), s).await
    }
}

/// Parse a value from a string in context of a received message.
///
/// This trait is a superset of [`std::str::FromStr`]. The
/// difference is that this trait aims to support serenity-specific Discord types like [`Member`]
/// or [`Message`].
///
/// Trait implementations may do network requests as part of their parsing procedure.
///
/// Useful for implementing argument parsing in command frameworks.
#[async_trait::async_trait]
pub trait ArgumentConvert: Sized {
    /// The associated error which can be returned from parsing.
    type Err;

    /// Parses a string `s` as a command parameter of this type.
    async fn convert(
        ctx: &Context,
        guild_id: Option<GuildId>,
        channel_id: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err>;
}

#[async_trait::async_trait]
impl<T: std::str::FromStr> ArgumentConvert for T {
    type Err = <T as std::str::FromStr>::Err;

    async fn convert(
        _: &Context,
        _: Option<GuildId>,
        _: Option<ChannelId>,
        s: &str,
    ) -> Result<Self, Self::Err> {
        T::from_str(s)
    }
}

// The following few parse_XXX methods are in here (parse.rs) because they need to be gated
// behind the model feature and it's just convenient to put them here for that

/// Retrieves IDs from "{channel ID}-{message ID}" (retrieved by shift-clicking on "Copy ID").
///
/// If the string is invalid, None is returned.
///
/// # Examples
/// ```rust
/// use serenity::model::prelude::*;
/// use serenity::utils::parse_message_id_pair;
///
/// assert_eq!(
///     parse_message_id_pair("673965002805477386-842482646604972082"),
///     Some((ChannelId(673965002805477386), MessageId(842482646604972082))),
/// );
/// assert_eq!(
///     parse_message_id_pair("673965002805477386-842482646604972082-472029906943868929"),
///     None,
/// );
/// ```
pub fn parse_message_id_pair(s: &str) -> Option<(ChannelId, MessageId)> {
    let mut parts = s.splitn(2, '-');
    let channel_id = ChannelId(parts.next()?.parse().ok()?);
    let message_id = MessageId(parts.next()?.parse().ok()?);
    Some((channel_id, message_id))
}

/// Retrieves guild, channel, and message ID from a message URL.
///
/// If the URL is malformed, None is returned.
///
/// # Examples
/// ```rust
/// use serenity::model::prelude::*;
/// use serenity::utils::parse_message_url;
///
/// assert_eq!(
///     parse_message_url(
///         "https://discord.com/channels/381880193251409931/381880193700069377/806164913558781963"
///     ),
///     Some((
///         GuildId(381880193251409931),
///         ChannelId(381880193700069377),
///         MessageId(806164913558781963),
///     )),
/// );
/// assert_eq!(parse_message_url("https://google.com"), None);
/// ```
pub fn parse_message_url(s: &str) -> Option<(GuildId, ChannelId, MessageId)> {
    let mut parts = s.strip_prefix("https://discord.com/channels/")?.splitn(3, '/');
    let guild_id = GuildId(parts.next()?.parse().ok()?);
    let channel_id = ChannelId(parts.next()?.parse().ok()?);
    let message_id = MessageId(parts.next()?.parse().ok()?);
    Some((guild_id, channel_id, message_id))
}

/// Retrieves the username and discriminator out of a user tag (`name#discrim`).
///
/// If the user tag is invalid, None is returned.
///
/// # Examples
/// ```rust
/// use serenity::utils::parse_user_tag;
///
/// assert_eq!(parse_user_tag("kangalioo#9108"), Some(("kangalioo", 9108)));
/// assert_eq!(parse_user_tag("kangalioo#10108"), None);
/// ```
pub fn parse_user_tag(s: &str) -> Option<(&str, u16)> {
    let pound_sign = s.find('#')?;
    let name = &s[..pound_sign];
    let discrim = s[(pound_sign + 1)..].parse::<u16>().ok()?;
    if discrim > 9999 {
        return None;
    }
    Some((name, discrim))
}
