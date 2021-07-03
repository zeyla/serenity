use std::{collections::HashMap, hash::Hash};

use serde::de::Error as DeError;
use serde::de::MapAccess;
use serde::ser::{Serialize, SerializeSeq, Serializer};

#[cfg(all(feature = "cache", feature = "model"))]
use super::permissions::Permissions;
use super::prelude::*;
#[cfg(all(feature = "cache", feature = "model"))]
use crate::cache::Cache;
#[cfg(feature = "cache")]
use crate::internal::prelude::*;
#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
use crate::model::interactions::ApplicationCommandInteractionDataOption;

pub fn default_true() -> bool {
    true
}

pub fn deserialize_emojis<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<EmojiId, Emoji>, D::Error> {
    let vec: Vec<Emoji> = Deserialize::deserialize(deserializer)?;
    let mut emojis = HashMap::new();

    for emoji in vec {
        emojis.insert(emoji.id, emoji);
    }

    Ok(emojis)
}

pub fn serialize_emojis<S: Serializer>(
    emojis: &HashMap<EmojiId, Emoji>,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(emojis.len()))?;

    for emoji in emojis.values() {
        seq.serialize_element(emoji)?;
    }

    seq.end()
}

pub fn deserialize_guild_channels<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<ChannelId, GuildChannel>, D::Error> {
    let vec: Vec<GuildChannel> = Deserialize::deserialize(deserializer)?;
    let mut map = HashMap::new();

    for channel in vec {
        map.insert(channel.id, channel);
    }

    Ok(map)
}

pub fn deserialize_members<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<UserId, Member>, D::Error> {
    let vec: Vec<Member> = Deserialize::deserialize(deserializer)?;
    let mut members = HashMap::new();

    for member in vec {
        let user_id = member.user.id;

        members.insert(user_id, member);
    }

    Ok(members)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_partial_members_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<UserId, PartialMember>, D::Error> {
    let map: HashMap<UserId, PartialMember> = Deserialize::deserialize(deserializer)?;

    Ok(map)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_users<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<UserId, User>, D::Error> {
    let map: HashMap<UserId, User> = Deserialize::deserialize(deserializer)?;

    Ok(map)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_roles_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<RoleId, Role>, D::Error> {
    let map: HashMap<RoleId, Role> = Deserialize::deserialize(deserializer)?;

    Ok(map)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_channels_map<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<ChannelId, PartialChannel>, D::Error> {
    let map: HashMap<ChannelId, PartialChannel> = Deserialize::deserialize(deserializer)?;

    Ok(map)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_options<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<Vec<ApplicationCommandInteractionDataOption>, D::Error> {
    let options: Vec<ApplicationCommandInteractionDataOption> =
        Deserialize::deserialize(deserializer)?;

    Ok(options)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
pub fn deserialize_options_with_resolved<'de, D: Deserializer<'de>>(
    deserializer: D,
    resolved: &ApplicationCommandInteractionDataResolved,
) -> StdResult<Vec<ApplicationCommandInteractionDataOption>, D::Error> {
    let mut options: Vec<ApplicationCommandInteractionDataOption> =
        Deserialize::deserialize(deserializer)?;

    for option in options.iter_mut() {
        loop_resolved(option, resolved);
    }

    Ok(options)
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
fn set_resolved(
    mut options: &mut ApplicationCommandInteractionDataOption,
    resolved: &ApplicationCommandInteractionDataResolved,
) {
    if let Some(ref value) = options.value {
        let string = value.as_str();

        options.resolved = match options.kind {
            ApplicationCommandOptionType::User => {
                let id = &UserId(string.unwrap().parse().unwrap());

                let user = resolved.users.get(id).unwrap().to_owned();
                let member = resolved.members.get(id).map(|m| m.to_owned());

                Some(ApplicationCommandInteractionDataOptionValue::User(user, member))
            },
            ApplicationCommandOptionType::Role => {
                let id = &RoleId(string.unwrap().parse().unwrap());

                let role = resolved.roles.get(id).unwrap().to_owned();

                Some(ApplicationCommandInteractionDataOptionValue::Role(role))
            },
            ApplicationCommandOptionType::Channel => {
                let id = &ChannelId(string.unwrap().parse().unwrap());

                let channel = resolved.channels.get(id).unwrap().to_owned();

                Some(ApplicationCommandInteractionDataOptionValue::Channel(channel))
            },
            ApplicationCommandOptionType::Mentionable => {
                let id: u64 = string.unwrap().parse().unwrap();

                if let Some(user) = resolved.users.get(&UserId(id)) {
                    let user = user.to_owned();
                    let member = resolved.members.get(&UserId(id)).map(|m| m.to_owned());

                    Some(ApplicationCommandInteractionDataOptionValue::User(user, member))
                } else {
                    let role = resolved.roles.get(&RoleId(id)).unwrap().to_owned();

                    Some(ApplicationCommandInteractionDataOptionValue::Role(role))
                }
            },
            ApplicationCommandOptionType::String => Some(
                ApplicationCommandInteractionDataOptionValue::String(string.unwrap().to_owned()),
            ),
            ApplicationCommandOptionType::Integer => {
                Some(ApplicationCommandInteractionDataOptionValue::Integer(value.as_i64().unwrap()))
            },
            ApplicationCommandOptionType::Boolean => Some(
                ApplicationCommandInteractionDataOptionValue::Boolean(value.as_bool().unwrap()),
            ),
            _ => None,
        }
    }
}

#[cfg(all(feature = "unstable_discord_api", feature = "model"))]
fn loop_resolved(
    options: &mut ApplicationCommandInteractionDataOption,
    resolved: &ApplicationCommandInteractionDataResolved,
) {
    set_resolved(options, resolved);

    for option in options.options.iter_mut() {
        loop_resolved(option, resolved);
    }
}

pub fn deserialize_presences<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<UserId, Presence>, D::Error> {
    let vec: Vec<Presence> = Deserialize::deserialize(deserializer)?;
    let mut presences = HashMap::new();

    for presence in vec {
        presences.insert(presence.user_id, presence);
    }

    Ok(presences)
}

pub fn serialize_presences<S: Serializer>(
    presences: &HashMap<UserId, Presence>,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(presences.len()))?;

    for presence in presences.values() {
        seq.serialize_element(presence)?;
    }

    seq.end()
}

pub fn deserialize_buttons<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<Vec<ActivityButton>, D::Error> {
    let labels: Vec<String> = Deserialize::deserialize(deserializer)?;
    let mut buttons = vec![];

    for label in labels {
        buttons.push(ActivityButton {
            label,
            url: "".to_owned(),
        });
    }

    Ok(buttons)
}

pub fn deserialize_private_channels<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<ChannelId, Channel>, D::Error> {
    let vec: Vec<Channel> = Deserialize::deserialize(deserializer)?;
    let mut private_channels = HashMap::new();

    for private_channel in vec {
        let id = match private_channel {
            Channel::Private(ref channel) => channel.id,
            Channel::Guild(_) => unreachable!("Guild private channel decode"),
            Channel::Category(_) => unreachable!("Channel category private channel decode"),
        };

        private_channels.insert(id, private_channel);
    }

    Ok(private_channels)
}

pub fn serialize_private_channels<S: Serializer>(
    private_channels: &HashMap<ChannelId, Channel>,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(private_channels.len()))?;

    for private_channel in private_channels.values() {
        seq.serialize_element(private_channel)?;
    }

    seq.end()
}

pub fn deserialize_roles<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<RoleId, Role>, D::Error> {
    let vec: Vec<Role> = Deserialize::deserialize(deserializer)?;
    let mut roles = HashMap::new();

    for role in vec {
        roles.insert(role.id, role);
    }

    Ok(roles)
}

pub fn serialize_roles<S: Serializer>(
    roles: &HashMap<RoleId, Role>,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(roles.len()))?;

    for role in roles.values() {
        seq.serialize_element(role)?;
    }

    seq.end()
}

pub fn deserialize_single_recipient<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<User, D::Error> {
    let mut users: Vec<User> = Deserialize::deserialize(deserializer)?;
    let user = if users.is_empty() {
        return Err(DeError::custom("Expected a single recipient"));
    } else {
        users.remove(0)
    };

    Ok(user)
}

pub fn serialize_single_recipient<S: Serializer>(
    user: &User,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(1))?;

    seq.serialize_element(user)?;

    seq.end()
}

pub fn deserialize_u16<'de, D: Deserializer<'de>>(deserializer: D) -> StdResult<u16, D::Error> {
    deserializer.deserialize_any(U16Visitor)
}

pub fn deserialize_u64<'de, D: Deserializer<'de>>(deserializer: D) -> StdResult<u64, D::Error> {
    deserializer.deserialize_any(U64Visitor)
}

#[allow(clippy::trivially_copy_pass_by_ref)]
pub fn serialize_u64<S: Serializer>(data: &u64, ser: S) -> StdResult<S::Ok, S::Error> {
    ser.serialize_str(&data.to_string())
}

pub fn deserialize_voice_states<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> StdResult<HashMap<UserId, VoiceState>, D::Error> {
    let vec: Vec<VoiceState> = Deserialize::deserialize(deserializer)?;
    let mut voice_states = HashMap::new();

    for voice_state in vec {
        voice_states.insert(voice_state.user_id, voice_state);
    }

    Ok(voice_states)
}

pub fn serialize_gen_map<K: Eq + Hash, S: Serializer, V: Serialize>(
    map: &HashMap<K, V>,
    serializer: S,
) -> StdResult<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(map.len()))?;

    for value in map.values() {
        seq.serialize_element(&value)?;
    }

    seq.end()
}

/// Tries to find a user's permissions using the cache.
/// Unlike [`user_has_perms`], this function will return `true` even when
/// the permissions are not in the cache.
#[cfg(all(feature = "cache", feature = "model"))]
#[inline]
pub async fn user_has_perms_cache(
    cache: impl AsRef<Cache>,
    channel_id: ChannelId,
    guild_id: Option<GuildId>,
    permissions: Permissions,
) -> Result<()> {
    if match user_has_perms(cache, channel_id, guild_id, permissions).await {
        Err(Error::Model(err)) => err.is_cache_err(),
        result => result?,
    } {
        Ok(())
    } else {
        Err(Error::Model(ModelError::InvalidPermissions(permissions)))
    }
}

#[cfg(all(feature = "cache", feature = "model"))]
pub async fn user_has_perms(
    cache: impl AsRef<Cache>,
    channel_id: ChannelId,
    guild_id: Option<GuildId>,
    mut permissions: Permissions,
) -> Result<bool> {
    let cache = cache.as_ref();

    let channel = match cache.channel(channel_id).await {
        Some(channel) => channel,
        None => return Err(Error::Model(ModelError::ChannelNotFound)),
    };

    // Both users in DMs, all users in groups, and maybe all channels in categories
    // will have the same permissions.
    //
    // The only exception to this is when the current user is blocked by
    // the recipient in a DM channel, preventing the current user
    // from sending messages.
    //
    // Since serenity can't _reasonably_ check and keep track of these,
    // just assume that all permissions are granted and return `true`.
    let (guild_id, guild_channel) = match channel {
        Channel::Guild(channel) => (channel.guild_id, channel),
        Channel::Category(_) => return Ok(true),
        Channel::Private(_) => match guild_id {
            Some(_) => return Err(Error::Model(ModelError::InvalidChannelType)),
            None => return Ok(true),
        },
    };

    let guild = match cache.guild(guild_id).await {
        Some(guild) => guild,
        None => return Err(Error::Model(ModelError::GuildNotFound)),
    };

    let member = match guild.members.get(&cache.current_user().await.id) {
        Some(member) => member,
        None => return Err(Error::Model(ModelError::MemberNotFound)),
    };

    let perms = guild.user_permissions_in(&guild_channel, member)?;

    permissions.remove(perms);

    Ok(permissions.is_empty())
}

macro_rules! num_visitors {
    ($($visitor:ident: $type:ty),*) => {
        $(
            #[derive(Debug)]
            pub struct $visitor;

            impl<'de> Visitor<'de> for $visitor {
                type Value = $type;

                fn expecting(&self, formatter: &mut Formatter<'_>) -> FmtResult {
                    formatter.write_str("identifier")
                }

                fn visit_str<E: DeError>(self, v: &str) -> StdResult<Self::Value, E> {
                    v.parse::<$type>().map_err(|_| {
                        let mut s = String::with_capacity(32);
                        s.push_str("Unknown ");
                        s.push_str(stringify!($type));
                        s.push_str(" value: ");
                        s.push_str(v);

                        DeError::custom(s)
                    })
                }

                fn visit_i64<E: DeError>(self, v: i64) -> StdResult<Self::Value, E> { Ok(v as $type) }

                fn visit_u64<E: DeError>(self, v: u64) -> StdResult<Self::Value, E> { Ok(v as $type) }

                // This is called when serde_json's `arbitrary_precision` feature is enabled.
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> StdResult<Self::Value, A::Error> {
                    struct Id {
                        num: $type,
                    }

                    struct StrVisitor;

                    impl<'de> Visitor<'de> for StrVisitor {
                        type Value = $type;

                        fn expecting(&self, formatter: &mut Formatter<'_>) -> FmtResult {
                            formatter.write_str("string")
                        }

                        fn visit_str<E: DeError>(self, s: &str) -> StdResult<Self::Value, E> { s.parse().map_err(E::custom) }
                        fn visit_string<E: DeError>(self, s: String) -> StdResult<Self::Value, E> { s.parse().map_err(E::custom) }
                    }

                    impl<'de> Deserialize<'de> for Id {
                        fn deserialize<D: Deserializer<'de>>(des: D) -> StdResult<Self, D::Error> {
                            Ok(Id { num: des.deserialize_str(StrVisitor)? })
                        }
                    }

                    map.next_value::<Id>().map(|id| id.num)
                }
            }
        )*
    }
}

num_visitors!(U16Visitor: u16, U32Visitor: u32, U64Visitor: u64);
