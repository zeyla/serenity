//! A set of utilities to help with common use cases that are not required to
//! fully use the library.

#[cfg(all(feature = "client", feature = "cache"))]
mod argument_convert;
mod colour;
mod custom_message;
mod message_builder;

#[cfg(all(feature = "client", feature = "cache"))]
pub use argument_convert::*;

pub use self::{
    colour::Colour,
    custom_message::CustomMessage,
    message_builder::{Content, ContentModifier, EmbedMessageBuilding, MessageBuilder},
};
pub type Color = Colour;

#[cfg(feature = "cache")]
use std::str::FromStr;
use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::File,
    hash::{BuildHasher, Hash},
    io::Read,
    path::Path,
};

#[cfg(feature = "cache")]
use crate::cache::Cache;
use crate::internal::prelude::*;
#[cfg(feature = "cache")]
use crate::model::channel::Channel;
#[cfg(feature = "cache")]
use crate::model::id::{ChannelId, GuildId, RoleId, UserId};
use crate::model::{id::EmojiId, misc::EmojiIdentifier};

/// Converts a HashMap into a final [`serde_json::Map`] representation.
pub fn hashmap_to_json_map<H, T>(map: HashMap<T, Value, H>) -> Map<String, Value>
where
    H: BuildHasher,
    T: Eq + Hash + ToString,
{
    let mut json_map = Map::new();

    for (key, value) in map {
        json_map.insert(key.to_string(), value);
    }

    json_map
}

/// Retrieves the "code" part of an invite out of a URL.
///
/// # Examples
///
/// Two formats of [invite][`RichInvite`] codes are supported, both regardless of protocol prefix.
/// Some examples:
///
/// 1. Retrieving the code from the URL `"https://discord.gg/0cDvIgU2voY8RSYL"`:
///
/// ```rust
/// use serenity::utils;
///
/// let url = "https://discord.gg/0cDvIgU2voY8RSYL";
///
/// assert_eq!(utils::parse_invite(url), "0cDvIgU2voY8RSYL");
/// ```
///
/// 2. Retrieving the code from the URL `"http://discord.com/invite/0cDvIgU2voY8RSYL"`:
///
/// ```rust
/// use serenity::utils;
///
/// let url = "http://discord.com/invite/0cDvIgU2voY8RSYL";
///
/// assert_eq!(utils::parse_invite(url), "0cDvIgU2voY8RSYL");
/// ```
///
/// [`RichInvite`]: crate::model::invite::RichInvite
pub fn parse_invite(code: &str) -> &str {
    let code = code.trim_start_matches("http://").trim_start_matches("https://");
    let lower = code.to_lowercase();
    if lower.starts_with("discord.gg/") {
        &code[11..]
    } else if lower.starts_with("discord.com/invite/") {
        &code[19..]
    } else {
        code
    }
}

/// Retrieves an Id from a user mention.
///
/// If the mention is invalid, then [`None`] is returned.
///
/// # Examples
///
/// Retrieving an Id from a valid [`User`] mention:
///
/// ```rust
/// use serenity::utils::parse_username;
///
/// // regular username mention
/// assert_eq!(parse_username("<@114941315417899012>"), Some(114941315417899012));
///
/// // nickname mention
/// assert_eq!(parse_username("<@!114941315417899012>"), Some(114941315417899012));
/// ```
///
/// Asserting that an invalid username or nickname mention returns [`None`]:
///
/// ```rust
/// use serenity::utils::parse_username;
///
/// assert!(parse_username("<@1149413154aa17899012").is_none());
/// assert!(parse_username("<@!11494131541789a90b1c2").is_none());
/// ```
///
/// [`User`]: crate::model::user::User
pub fn parse_username(mention: impl AsRef<str>) -> Option<u64> {
    let mention = mention.as_ref();

    if mention.len() < 4 {
        return None;
    }

    if mention.starts_with("<@!") {
        let len = mention.len() - 1;
        mention[3..len].parse::<u64>().ok()
    } else if mention.starts_with("<@") {
        let len = mention.len() - 1;
        mention[2..len].parse::<u64>().ok()
    } else {
        None
    }
}

/// Retrieves an Id from a role mention.
///
/// If the mention is invalid, then [`None`] is returned.
///
/// # Examples
///
/// Retrieving an Id from a valid [`Role`] mention:
///
/// ```rust
/// use serenity::utils::parse_role;
///
/// assert_eq!(parse_role("<@&136107769680887808>"), Some(136107769680887808));
/// ```
///
/// Asserting that an invalid role mention returns [`None`]:
///
/// ```rust
/// use serenity::utils::parse_role;
///
/// assert!(parse_role("<@&136107769680887808").is_none());
/// ```
///
/// [`Role`]: crate::model::guild::Role
pub fn parse_role(mention: impl AsRef<str>) -> Option<u64> {
    let mention = mention.as_ref();

    if mention.len() < 4 {
        return None;
    }

    if mention.starts_with("<@&") && mention.ends_with('>') {
        let len = mention.len() - 1;
        mention[3..len].parse::<u64>().ok()
    } else {
        None
    }
}

/// Retrieves an Id from a channel mention.
///
/// If the channel mention is invalid, then [`None`] is returned.
///
/// # Examples
///
/// Retrieving an Id from a valid [`Channel`] mention:
///
/// ```rust
/// use serenity::utils::parse_channel;
///
/// assert_eq!(parse_channel("<#81384788765712384>"), Some(81384788765712384));
/// ```
///
/// Asserting that an invalid channel mention returns [`None`]:
///
/// ```rust
/// use serenity::utils::parse_channel;
///
/// assert!(parse_channel("<#!81384788765712384>").is_none());
/// assert!(parse_channel("<#81384788765712384").is_none());
/// ```
///
/// [`Channel`]: crate::model::channel::Channel
pub fn parse_channel(mention: impl AsRef<str>) -> Option<u64> {
    let mention = mention.as_ref();

    if mention.len() < 4 {
        return None;
    }

    if mention.starts_with("<#") && mention.ends_with('>') {
        let len = mention.len() - 1;
        mention[2..len].parse::<u64>().ok()
    } else {
        None
    }
}

/// Retrieve the ID number out of a channel, role, or user mention.
///
/// If the mention is invalid, [`None`] is returned.
///
/// # Examples
///
/// ```rust
/// use serenity::utils::parse_mention;
///
/// assert_eq!(parse_mention("<@136510335967297536>"), Some(136510335967297536));
/// assert_eq!(parse_mention("<@&137235212097683456>"), Some(137235212097683456));
/// assert_eq!(parse_mention("<#137234234728251392>"), Some(137234234728251392));
/// ```
pub fn parse_mention(mention: impl AsRef<str>) -> Option<u64> {
    let mention = mention.as_ref();

    if mention.starts_with("<@&") {
        parse_role(mention)
    } else if mention.starts_with("<@") || mention.starts_with("<@!") {
        parse_username(mention)
    } else if mention.starts_with("<#") {
        parse_channel(mention)
    } else {
        None
    }
}

/// Retrieves the animated state, name and Id from an emoji mention, in the form of an
/// [`EmojiIdentifier`].
///
/// If the emoji usage is invalid, then [`None`] is returned.
///
/// # Examples
///
/// Ensure that a valid [`Emoji`] usage is correctly parsed:
///
/// ```rust
/// use serenity::model::id::{EmojiId, GuildId};
/// use serenity::model::misc::EmojiIdentifier;
/// use serenity::utils::parse_emoji;
///
/// let expected = EmojiIdentifier {
///     animated: false,
///     id: EmojiId(302516740095606785),
///     name: "smugAnimeFace".to_string(),
/// };
///
/// assert_eq!(parse_emoji("<:smugAnimeFace:302516740095606785>").unwrap(), expected);
/// ```
///
/// Asserting that an invalid emoji usage returns [`None`]:
///
/// ```rust
/// use serenity::utils::parse_emoji;
///
/// assert!(parse_emoji("<:smugAnimeFace:302516740095606785").is_none());
/// ```
///
/// [`Emoji`]: crate::model::guild::Emoji
pub fn parse_emoji(mention: impl AsRef<str>) -> Option<EmojiIdentifier> {
    let mention = mention.as_ref();

    let len = mention.len();

    if !(6..=56).contains(&len) {
        return None;
    }

    if (mention.starts_with("<:") || mention.starts_with("<a:")) && mention.ends_with('>') {
        let mut name = String::default();
        let mut id = String::default();
        let animated = &mention[1..3] == "a:";

        let start = if animated { 3 } else { 2 };

        for (i, x) in mention[start..].chars().enumerate() {
            if x == ':' {
                let from = i + start + 1;

                for y in mention[from..].chars() {
                    if y == '>' {
                        break;
                    } else {
                        id.push(y);
                    }
                }

                break;
            } else {
                name.push(x);
            }
        }

        match id.parse::<u64>() {
            Ok(x) => Some(EmojiIdentifier {
                animated,
                name,
                id: EmojiId(x),
            }),
            _ => None,
        }
    } else {
        None
    }
}

/// Reads an image from a path and encodes it into base64.
///
/// This can be used for methods like [`EditProfile::avatar`].
///
/// # Examples
///
/// Reads an image located at `./cat.png` into a base64-encoded string:
///
/// ```rust,no_run
/// use serenity::utils;
///
/// let image = utils::read_image("./cat.png").expect("Failed to read image");
/// ```
///
/// # Errors
///
/// Returns an [`Error::Io`] if the path does not exist.
///
/// [`EditProfile::avatar`]: crate::builder::EditProfile::avatar
/// [`Error::Io`]: crate::error::Error::Io
#[inline]
pub fn read_image<P: AsRef<Path>>(path: P) -> Result<String> {
    _read_image(path.as_ref())
}

fn _read_image(path: &Path) -> Result<String> {
    let mut v = Vec::default();
    let mut f = File::open(path)?;

    // errors here are intentionally ignored
    #[allow(clippy::let_underscore_must_use)]
    let _ = f.read_to_end(&mut v);

    let b64 = base64::encode(&v);
    let ext = if path.extension() == Some(OsStr::new("png")) { "png" } else { "jpg" };

    Ok(format!("data:image/{};base64,{}", ext, b64))
}

/// Turns a string into a vector of string arguments, splitting by spaces, but
/// parsing content within quotes as one individual argument.
///
/// # Examples
///
/// Parsing two quoted commands:
///
/// ```rust
/// use serenity::utils::parse_quotes;
///
/// let command = r#""this is the first" "this is the second""#;
/// let expected = vec!["this is the first".to_string(), "this is the second".to_string()];
///
/// assert_eq!(parse_quotes(command), expected);
/// ```
///
/// ```rust
/// use serenity::utils::parse_quotes;
///
/// let command = r#""this is a quoted command that doesn't have an ending quotation"#;
/// let expected =
///     vec!["this is a quoted command that doesn't have an ending quotation".to_string()];
///
/// assert_eq!(parse_quotes(command), expected);
/// ```
pub fn parse_quotes(s: impl AsRef<str>) -> Vec<String> {
    let s = s.as_ref();
    let mut args = vec![];
    let mut in_string = false;
    let mut escaping = false;
    let mut current_str = String::default();

    for x in s.chars() {
        if in_string {
            if x == '\\' && !escaping {
                escaping = true;
            } else if x == '"' && !escaping {
                if !current_str.is_empty() {
                    args.push(current_str);
                }

                current_str = String::default();
                in_string = false;
            } else {
                current_str.push(x);
                escaping = false;
            }
        } else if x == ' ' {
            if !current_str.is_empty() {
                args.push(current_str.clone());
            }

            current_str = String::default();
        } else if x == '"' {
            if !current_str.is_empty() {
                args.push(current_str.clone());
            }

            in_string = true;
            current_str = String::default();
        } else {
            current_str.push(x);
        }
    }

    if !current_str.is_empty() {
        args.push(current_str);
    }

    args
}

/// Calculates the Id of the shard responsible for a guild, given its Id and
/// total number of shards used.
///
/// # Examples
///
/// Retrieve the Id of the shard for a guild with Id `81384788765712384`, using
/// 17 shards:
///
/// ```rust
/// use serenity::utils;
///
/// assert_eq!(utils::shard_id(81384788765712384 as u64, 17), 7);
/// ```
#[inline]
pub fn shard_id(guild_id: impl Into<u64>, shard_count: u64) -> u64 {
    (guild_id.into() >> 22) % shard_count
}

/// Struct that allows to alter [`content_safe`]'s behaviour.
#[cfg(feature = "cache")]
#[derive(Clone, Debug)]
pub struct ContentSafeOptions {
    clean_role: bool,
    clean_user: bool,
    clean_channel: bool,
    clean_here: bool,
    clean_everyone: bool,
    show_discriminator: bool,
    guild_reference: Option<GuildId>,
}

#[cfg(feature = "cache")]
impl ContentSafeOptions {
    pub fn new() -> Self {
        ContentSafeOptions::default()
    }

    /// [`content_safe`] will replace role mentions (`<@&{id}>`) with its name
    /// prefixed with `@` (`@rolename`) or with `@deleted-role` if the
    /// identifier is invalid.
    pub fn clean_role(mut self, b: bool) -> Self {
        self.clean_role = b;

        self
    }

    /// If set to true, [`content_safe`] will replace user mentions
    /// (`<@!{id}>` or `<@{id}>`) with the user's name prefixed with `@`
    /// (`@username`) or with `@invalid-user` if the identifier is invalid.
    pub fn clean_user(mut self, b: bool) -> Self {
        self.clean_user = b;

        self
    }

    /// If set to true, [`content_safe`] will replace channel mentions
    /// (`<#{id}>`) with the channel's name prefixed with `#`
    /// (`#channelname`) or with `#deleted-channel` if the identifier is
    /// invalid.
    pub fn clean_channel(mut self, b: bool) -> Self {
        self.clean_channel = b;

        self
    }

    /// If set to true, if [`content_safe`] replaces a user mention it will
    /// add their four digit discriminator with a preceeding `#`,
    /// turning `@username` to `@username#discriminator`.
    pub fn show_discriminator(mut self, b: bool) -> Self {
        self.show_discriminator = b;

        self
    }

    /// If set, [`content_safe`] will replace a user mention with the user's
    /// display name in passed `guild`.
    pub fn display_as_member_from<G: Into<GuildId>>(mut self, guild: G) -> Self {
        self.guild_reference = Some(guild.into());

        self
    }

    /// If set, [`content_safe`] will replace `@here` with a non-pinging
    /// alternative.
    pub fn clean_here(mut self, b: bool) -> Self {
        self.clean_here = b;

        self
    }

    /// If set, [`content_safe`] will replace `@everyone` with a non-pinging
    /// alternative.
    pub fn clean_everyone(mut self, b: bool) -> Self {
        self.clean_everyone = b;

        self
    }
}

#[cfg(feature = "cache")]
impl Default for ContentSafeOptions {
    /// Instantiates with all options set to `true`.
    fn default() -> Self {
        ContentSafeOptions {
            clean_role: true,
            clean_user: true,
            clean_channel: true,
            clean_here: true,
            clean_everyone: true,
            show_discriminator: true,
            guild_reference: None,
        }
    }
}

#[cfg(feature = "cache")]
#[inline]
async fn clean_roles(cache: impl AsRef<Cache>, s: &mut String) {
    let mut progress = 0;

    while let Some(mut mention_start) = s[progress..].find("<@&") {
        mention_start += progress;

        if let Some(mut mention_end) = s[mention_start..].find('>') {
            mention_end += mention_start;
            mention_start += "<@&".len();

            if let Ok(id) = RoleId::from_str(&s[mention_start..mention_end]) {
                let to_replace = format!("<@&{}>", &s[mention_start..mention_end]);

                *s = if let Some(role) = id.to_role_cached(&cache).await {
                    s.replace(&to_replace, &format!("@{}", &role.name))
                } else {
                    s.replace(&to_replace, "@deleted-role")
                };
            } else {
                let id = &s[mention_start..mention_end].to_string();

                if !id.is_empty() && id.as_bytes().iter().all(u8::is_ascii_digit) {
                    let to_replace = format!("<@&{}>", id);

                    *s = s.replace(&to_replace, "@deleted-role");
                } else {
                    progress = mention_end;
                }
            }
        } else {
            break;
        }
    }
}

#[cfg(feature = "cache")]
#[inline]
async fn clean_channels(cache: &impl AsRef<Cache>, s: &mut String) {
    let mut progress = 0;

    while let Some(mut mention_start) = s[progress..].find("<#") {
        mention_start += progress;

        if let Some(mut mention_end) = s[mention_start..].find('>') {
            mention_end += mention_start;
            mention_start += "<#".len();

            if let Ok(id) = ChannelId::from_str(&s[mention_start..mention_end]) {
                let to_replace = format!("<#{}>", &s[mention_start..mention_end]);

                *s = if let Some(Channel::Guild(channel)) = id.to_channel_cached(&cache).await {
                    let replacement = format!("#{}", &channel.name);
                    s.replace(&to_replace, &replacement)
                } else {
                    s.replace(&to_replace, "#deleted-channel")
                };
            } else {
                let id = &s[mention_start..mention_end].to_string();

                if !id.is_empty() && id.as_bytes().iter().all(u8::is_ascii_digit) {
                    let to_replace = format!("<#{}>", id);

                    *s = s.replace(&to_replace, "#deleted-channel");
                } else {
                    progress = mention_end;
                }
            }
        } else {
            break;
        }
    }
}

#[cfg(feature = "cache")]
#[inline]
async fn clean_users(
    cache: &impl AsRef<Cache>,
    s: &mut String,
    show_discriminator: bool,
    guild: Option<GuildId>,
) {
    let cache = cache.as_ref();
    let mut progress = 0;

    while let Some(mut mention_start) = s[progress..].find("<@") {
        mention_start += progress;

        if let Some(mut mention_end) = s[mention_start..].find('>') {
            mention_end += mention_start;
            mention_start += "<@".len();

            let has_exclamation =
                if s[mention_start..].as_bytes().get(0).map_or(false, |c| *c == b'!') {
                    mention_start += "!".len();

                    true
                } else {
                    false
                };

            if let Ok(id) = UserId::from_str(&s[mention_start..mention_end]) {
                let replacement = if let Some(guild_id) = guild {
                    if let Some(guild) = cache.guild(&guild_id).await {
                        if let Some(member) = guild.members.get(&id) {
                            if show_discriminator {
                                format!("@{}", member.distinct())
                            } else {
                                format!("@{}", member.display_name())
                            }
                        } else {
                            "@invalid-user".to_string()
                        }
                    } else {
                        "@invalid-user".to_string()
                    }
                } else if let Some(user) = cache.user(id).await {
                    if show_discriminator {
                        format!("@{}#{:04}", user.name, user.discriminator)
                    } else {
                        format!("@{}", user.name)
                    }
                } else {
                    "@invalid-user".to_string()
                };

                let code_start = if has_exclamation { "<@!" } else { "<@" };
                let to_replace = format!("{}{}>", code_start, &s[mention_start..mention_end]);

                *s = s.replace(&to_replace, &replacement)
            } else {
                let id = &s[mention_start..mention_end].to_string();

                if !id.is_empty() && id.as_bytes().iter().all(u8::is_ascii_digit) {
                    let code_start = if has_exclamation { "<@!" } else { "<@" };
                    let to_replace = format!("{}{}>", code_start, id);

                    *s = s.replace(&to_replace, "@invalid-user");
                } else {
                    progress = mention_end;
                }
            }
        } else {
            break;
        }
    }
}

/// Transforms role, channel, user, `@everyone` and `@here` mentions
/// into raw text by using the [`Cache`] only.
///
/// [`ContentSafeOptions`] decides what kind of mentions should be filtered
/// and how the raw-text will be displayed.
///
/// # Examples
///
/// Sanitise an `@everyone` mention.
///
/// ```rust
/// # use std::sync::Arc;
/// # use serenity::client::Cache;
/// # use tokio::sync::RwLock;
/// #
/// # async fn run() {
/// # let cache = Cache::default();
/// use serenity::utils::{content_safe, ContentSafeOptions};
///
/// let with_mention = "@everyone";
/// let without_mention = content_safe(&cache, &with_mention, &ContentSafeOptions::default()).await;
///
/// assert_eq!("@\u{200B}everyone".to_string(), without_mention);
/// # }
/// ```
///
/// [`Cache`]: crate::cache::Cache
#[cfg(feature = "cache")]
pub async fn content_safe(
    cache: impl AsRef<Cache>,
    s: impl AsRef<str>,
    options: &ContentSafeOptions,
) -> String {
    let mut content = s.as_ref().to_string();

    if options.clean_role {
        clean_roles(&cache, &mut content).await;
    }

    if options.clean_channel {
        clean_channels(&cache, &mut content).await;
    }

    if options.clean_user {
        clean_users(&cache, &mut content, options.show_discriminator, options.guild_reference)
            .await;
    }

    if options.clean_here {
        content = content.replace("@here", "@\u{200B}here");
    }

    if options.clean_everyone {
        content = content.replace("@everyone", "@\u{200B}everyone");
    }

    content
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::non_ascii_literal)]
mod test {
    use super::*;
    #[cfg(feature = "cache")]
    use crate::cache::Cache;

    #[test]
    fn test_invite_parser() {
        assert_eq!(parse_invite("https://discord.gg/abc"), "abc");
        assert_eq!(parse_invite("http://discord.gg/abc"), "abc");
        assert_eq!(parse_invite("discord.gg/abc"), "abc");
        assert_eq!(parse_invite("DISCORD.GG/ABC"), "ABC");
        assert_eq!(parse_invite("https://discord.com/invite/abc"), "abc");
        assert_eq!(parse_invite("http://discord.com/invite/abc"), "abc");
        assert_eq!(parse_invite("discord.com/invite/abc"), "abc");
    }

    #[test]
    fn test_username_parser() {
        assert_eq!(parse_username("<@12345>").unwrap(), 12_345);
        assert_eq!(parse_username("<@!12345>").unwrap(), 12_345);
    }

    #[test]
    fn role_parser() {
        assert_eq!(parse_role("<@&12345>").unwrap(), 12_345);
    }

    #[test]
    fn test_channel_parser() {
        assert_eq!(parse_channel("<#12345>").unwrap(), 12_345);
    }

    #[test]
    fn test_emoji_parser() {
        let emoji = parse_emoji("<:name:12345>").unwrap();
        assert_eq!(emoji.name, "name");
        assert_eq!(emoji.id, 12_345);
    }

    #[test]
    fn test_quote_parser() {
        let parsed = parse_quotes("a \"b c\" d\"e f\"  g");
        assert_eq!(parsed, ["a", "b c", "d", "e f", "g"]);
    }

    #[cfg(feature = "cache")]
    #[tokio::test]
    async fn test_content_safe() {
        use std::{collections::HashMap, sync::Arc};

        use chrono::{DateTime, Utc};

        use crate::model::{prelude::*, user::User, Permissions};

        let user = User {
            id: UserId(100000000000000000),
            avatar: None,
            bot: false,
            discriminator: 0000,
            name: "Crab".to_string(),
            public_flags: None,
        };

        #[allow(deprecated)]
        let mut guild = Guild {
            afk_channel_id: None,
            afk_timeout: 0,
            application_id: None,
            channels: HashMap::new(),
            default_message_notifications: DefaultMessageNotificationLevel::All,
            emojis: HashMap::new(),
            explicit_content_filter: ExplicitContentFilter::None,
            features: Vec::new(),
            icon: None,
            id: GuildId(381880193251409931),
            joined_at: DateTime::parse_from_str(
                "1983 Apr 13 12:09:14.274 +0000",
                "%Y %b %d %H:%M:%S%.3f %z",
            )
            .unwrap()
            .with_timezone(&Utc),
            large: false,
            member_count: 1,
            members: HashMap::new(),
            mfa_level: MfaLevel::None,
            name: "serenity".to_string(),
            owner_id: UserId(114941315417899012),
            presences: HashMap::new(),
            region: "Ferris Island".to_string(),
            roles: HashMap::new(),
            splash: None,
            discovery_splash: None,
            system_channel_id: None,
            system_channel_flags: Default::default(),
            rules_channel_id: None,
            public_updates_channel_id: None,
            verification_level: VerificationLevel::None,
            voice_states: HashMap::new(),
            description: None,
            premium_tier: PremiumTier::Tier0,
            premium_subscription_count: 0,
            banner: None,
            vanity_url_code: Some("bruhmoment1".to_string()),
            preferred_locale: "en-US".to_string(),
            welcome_screen: None,
            approximate_member_count: None,
            approximate_presence_count: None,
            nsfw: false,
            nsfw_level: NsfwLevel::Default,
            max_video_channel_users: None,
            max_presences: None,
            max_members: None,
            widget_enabled: Some(false),
            widget_channel_id: None,
            stage_instances: vec![],
            threads: vec![],
        };

        let member = Member {
            deaf: false,
            guild_id: guild.id,
            joined_at: None,
            mute: false,
            nick: Some("Ferris".to_string()),
            roles: Vec::new(),
            user: user.clone(),
            pending: false,
            premium_since: None,
            #[cfg(feature = "unstable_discord_api")]
            permissions: None,
            avatar: None,
        };

        let role = Role {
            id: RoleId(333333333333333333),
            colour: Colour::ORANGE,
            guild_id: guild.id,
            hoist: true,
            managed: false,
            mentionable: true,
            name: "ferris-club-member".to_string(),
            permissions: Permissions::all(),
            position: 0,
            tags: RoleTags::default(),
        };

        let channel = GuildChannel {
            id: ChannelId(111880193700067777),
            bitrate: None,
            category_id: None,
            guild_id: guild.id,
            kind: ChannelType::Text,
            last_message_id: None,
            last_pin_timestamp: None,
            name: "general".to_string(),
            permission_overwrites: Vec::new(),
            position: 0,
            topic: None,
            user_limit: None,
            nsfw: false,
            slow_mode_rate: Some(0),
            rtc_region: None,
            video_quality_mode: None,
            message_count: None,
            member_count: None,
            thread_metadata: None,
            member: None,
            default_auto_archive_duration: None,
        };

        let cache = Arc::new(Cache::default());

        guild.members.insert(user.id, member.clone());
        guild.roles.insert(role.id, role.clone());
        cache.users.write().await.insert(user.id, user.clone());
        cache.guilds.write().await.insert(guild.id, guild.clone());
        cache.channels.write().await.insert(channel.id, channel.clone());

        let with_user_mentions = "<@!100000000000000000> <@!000000000000000000> <@123> <@!123> \
        <@!123123123123123123123> <@123> <@123123123123123123> <@!invalid> \
        <@invalid> <@日本語 한국어$§)[/__#\\(/&2032$§#> \
        <@!i)/==(<<>z/9080)> <@!1231invalid> <@invalid123> \
        <@123invalid> <@> <@ ";

        let without_user_mentions = "@Crab#0000 @invalid-user @invalid-user @invalid-user \
        @invalid-user @invalid-user @invalid-user <@!invalid> \
        <@invalid> <@日本語 한국어$§)[/__#\\(/&2032$§#> \
        <@!i)/==(<<>z/9080)> <@!1231invalid> <@invalid123> \
        <@123invalid> <@> <@ ";

        // User mentions
        let options = ContentSafeOptions::default();
        assert_eq!(without_user_mentions, content_safe(&cache, with_user_mentions, &options).await);

        let options = ContentSafeOptions::default();
        assert_eq!(
            format!("@{}#{:04}", user.name, user.discriminator),
            content_safe(&cache, "<@!100000000000000000>", &options).await
        );

        let options = ContentSafeOptions::default();
        assert_eq!(
            format!("@{}#{:04}", user.name, user.discriminator),
            content_safe(&cache, "<@100000000000000000>", &options).await
        );

        let options = options.show_discriminator(false);
        assert_eq!(
            format!("@{}", user.name),
            content_safe(&cache, "<@!100000000000000000>", &options).await
        );

        let options = options.show_discriminator(false);
        assert_eq!(
            format!("@{}", user.name),
            content_safe(&cache, "<@100000000000000000>", &options).await
        );

        let options = options.display_as_member_from(guild.id);
        assert_eq!(
            format!("@{}", member.nick.unwrap()),
            content_safe(&cache, "<@!100000000000000000>", &options).await
        );

        let options = options.clean_user(false);
        assert_eq!(with_user_mentions, content_safe(&cache, with_user_mentions, &options).await);

        // Channel mentions
        let with_channel_mentions = "<#> <#deleted-channel> #deleted-channel <#0> \
        #unsafe-club <#111880193700067777> <#ferrisferrisferris> \
        <#000000000000000000>";

        let without_channel_mentions = "<#> <#deleted-channel> #deleted-channel \
        #deleted-channel #unsafe-club #general <#ferrisferrisferris> \
        #deleted-channel";

        assert_eq!(
            without_channel_mentions,
            content_safe(&cache, with_channel_mentions, &options).await
        );

        let options = options.clean_channel(false);
        assert_eq!(
            with_channel_mentions,
            content_safe(&cache, with_channel_mentions, &options).await
        );

        // Role mentions
        let with_role_mentions = "<@&> @deleted-role <@&9829> \
        <@&333333333333333333> <@&000000000000000000>";

        let without_role_mentions = "<@&> @deleted-role @deleted-role \
        @ferris-club-member @deleted-role";

        assert_eq!(without_role_mentions, content_safe(&cache, with_role_mentions, &options).await);

        let options = options.clean_role(false);
        assert_eq!(with_role_mentions, content_safe(&cache, with_role_mentions, &options).await);

        // Everyone mentions
        let with_everyone_mention = "@everyone";

        let without_everyone_mention = "@\u{200B}everyone";

        assert_eq!(
            without_everyone_mention,
            content_safe(&cache, with_everyone_mention, &options).await
        );

        let options = options.clean_everyone(false);
        assert_eq!(
            with_everyone_mention,
            content_safe(&cache, with_everyone_mention, &options).await
        );

        // Here mentions
        let with_here_mention = "@here";

        let without_here_mention = "@\u{200B}here";

        assert_eq!(without_here_mention, content_safe(&cache, with_here_mention, &options).await);

        let options = options.clean_here(false);
        assert_eq!(with_here_mention, content_safe(&cache, with_here_mention, &options).await);
    }
}
