use async_tungstenite::tungstenite::Message;
use futures::channel::mpsc::{TrySendError, UnboundedSender as Sender};

use super::{ChunkGuildFilter, ShardClientMessage, ShardRunnerMessage};
#[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
use crate::collector::ComponentInteractionFilter;
#[cfg(feature = "collector")]
use crate::collector::{EventFilter, MessageFilter, ReactionFilter};
use crate::gateway::InterMessage;
use crate::model::prelude::*;

/// A lightweight wrapper around an mpsc sender.
///
/// This is used to cleanly communicate with a shard's respective
/// [`ShardRunner`]. This can be used for actions such as setting the activity
/// via [`Self::set_activity`] or shutting down via [`Self::shutdown_clean`].
///
/// [`ShardRunner`]: super::ShardRunner
#[derive(Clone, Debug)]
pub struct ShardMessenger {
    pub(crate) tx: Sender<InterMessage>,
}

impl ShardMessenger {
    /// Creates a new shard messenger.
    ///
    /// If you are using the [`Client`], you do not need to do this.
    ///
    /// [`Client`]: crate::Client
    #[inline]
    pub fn new(tx: Sender<InterMessage>) -> Self {
        Self {
            tx,
        }
    }

    /// Requests that one or multiple [`Guild`]s be chunked.
    ///
    /// This will ask the gateway to start sending member chunks for large
    /// guilds (250 members+). If a guild is over 250 members, then a full
    /// member list will not be downloaded, and must instead be requested to be
    /// sent in "chunks" containing members.
    ///
    /// Member chunks are sent as the [`Event::GuildMembersChunk`] event. Each
    /// chunk only contains a partial amount of the total members.
    ///
    /// If the `cache` feature is enabled, the cache will automatically be
    /// updated with member chunks.
    ///
    /// # Examples
    ///
    /// Chunk a single guild by Id, limiting to 2000 [`Member`]s, and not
    /// specifying a query parameter:
    ///
    /// ```rust,no_run
    /// # use tokio::sync::Mutex;
    /// # use serenity::client::bridge::gateway::{GatewayIntents, ChunkGuildFilter};
    /// # use serenity::gateway::Shard;
    /// # use std::sync::Arc;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let mutex = Arc::new(Mutex::new("".to_string()));
    /// #
    /// #     let mut shard = Shard::new(mutex.clone(), "", [0u64, 1u64],
    /// #                                GatewayIntents::all()).await?;
    /// #
    /// use serenity::model::id::GuildId;
    ///
    /// shard.chunk_guild(GuildId(81384788765712384), Some(2000), ChunkGuildFilter::None, None);
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// Chunk a single guild by Id, limiting to 20 members, specifying a
    /// query parameter of `"do"` and a nonce of `"request"`:
    ///
    /// ```rust,no_run
    /// # use tokio::sync::Mutex;
    /// # use serenity::client::bridge::gateway::{GatewayIntents, ChunkGuildFilter};
    /// # use serenity::gateway::Shard;
    /// # use std::sync::Arc;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let mutex = Arc::new(Mutex::new("".to_string()));
    /// #
    /// #     let mut shard = Shard::new(mutex.clone(), "", [0u64, 1u64],
    /// #                                GatewayIntents::all()).await?;
    /// #
    /// use serenity::model::id::GuildId;
    ///
    /// shard.chunk_guild(GuildId(81384788765712384), Some(20), ChunkGuildFilter::Query("do".to_owned()), Some("request"));
    /// #     Ok(())
    /// # }
    /// ```
    pub fn chunk_guild(
        &self,
        guild_id: GuildId,
        limit: Option<u16>,
        filter: ChunkGuildFilter,
        nonce: Option<String>,
    ) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::ChunkGuild {
            guild_id,
            limit,
            filter,
            nonce,
        });
    }

    /// Sets the user's current activity, if any.
    ///
    /// Other presence settings are maintained.
    ///
    /// # Examples
    ///
    /// Setting the current activity to playing `"Heroes of the Storm"`:
    ///
    /// ```rust,no_run
    /// # use tokio::sync::Mutex;
    /// # use serenity::gateway::Shard;
    /// # use serenity::client::bridge::gateway::GatewayIntents;
    /// # use std::sync::Arc;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let mutex = Arc::new(Mutex::new("".to_string()));
    /// #
    /// #     let mut shard = Shard::new(mutex.clone(), "", [0u64, 1u64],
    /// #                                GatewayIntents::all()).await?;
    /// use serenity::model::gateway::Activity;
    ///
    /// shard.set_activity(Some(Activity::playing("Heroes of the Storm")));
    /// #     Ok(())
    /// # }
    /// ```
    pub fn set_activity(&self, activity: Option<Activity>) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetActivity(activity));
    }

    /// Sets the user's full presence information.
    ///
    /// Consider using the individual setters if you only need to modify one of
    /// these.
    ///
    /// # Examples
    ///
    /// Set the current user as playing `"Heroes of the Storm"` and being
    /// online:
    ///
    /// ```rust,ignore
    /// # use tokio::sync::Mutex;
    /// # use serenity::gateway::Shard;
    /// # use std::sync::Arc;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let mutex = Arc::new(Mutex::new("".to_string()));
    /// #
    /// #     let mut shard = Shard::new(mutex.clone(), "", [0u64, 1u64], None).await?;
    /// #
    /// use serenity::model::gateway::Activity;
    /// use serenity::model::user::OnlineStatus;
    ///
    /// let activity = Activity::playing("Heroes of the Storm");
    /// shard.set_presence(Some(activity), OnlineStatus::Online);
    /// #     Ok(())
    /// # }
    /// ```
    pub fn set_presence(&self, activity: Option<Activity>, mut status: OnlineStatus) {
        if status == OnlineStatus::Offline {
            status = OnlineStatus::Invisible;
        }

        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetPresence(status, activity));
    }

    /// Sets the user's current online status.
    ///
    /// Note that [`Offline`] is not a valid online status, so it is
    /// automatically converted to [`Invisible`].
    ///
    /// Other presence settings are maintained.
    ///
    /// # Examples
    ///
    /// Setting the current online status for the shard to [`DoNotDisturb`].
    ///
    /// ```rust,no_run
    /// # use tokio::sync::Mutex;
    /// # use serenity::gateway::Shard;
    /// # use serenity::client::bridge::gateway::GatewayIntents;
    /// # use std::sync::Arc;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let mutex = Arc::new(Mutex::new("".to_string()));
    /// #
    /// #     let mut shard = Shard::new(mutex.clone(), "", [0u64, 1u64],
    /// #                                GatewayIntents::all()).await?;
    /// #
    /// use serenity::model::user::OnlineStatus;
    ///
    /// shard.set_status(OnlineStatus::DoNotDisturb);
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// [`DoNotDisturb`]: OnlineStatus::DoNotDisturb
    /// [`Invisible`]: OnlineStatus::Invisible
    /// [`Offline`]: OnlineStatus::Offline
    pub fn set_status(&self, mut online_status: OnlineStatus) {
        if online_status == OnlineStatus::Offline {
            online_status = OnlineStatus::Invisible;
        }

        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetStatus(online_status));
    }

    /// Shuts down the websocket by attempting to cleanly close the
    /// connection.
    pub fn shutdown_clean(&self) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::Close(1000, None));
    }

    /// Sends a raw message over the WebSocket.
    ///
    /// The given message is not mutated in any way, and is sent as-is.
    ///
    /// You should only use this if you know what you're doing. If you're
    /// wanting to, for example, send a presence update, prefer the usage of
    /// the [`Self::set_presence`] method.
    pub fn websocket_message(&self, message: Message) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::Message(message));
    }

    /// Sends a message to the shard.
    /// # Errors
    ///
    /// Returns a [`TrySendError`] if the shard's receiver was closed.
    #[inline]
    pub fn send_to_shard(&self, msg: ShardRunnerMessage) -> Result<(), TrySendError<InterMessage>> {
        self.tx.unbounded_send(InterMessage::Client(Box::new(ShardClientMessage::Runner(msg))))
    }

    /// Sets a new filter for an event collector.
    #[inline]
    #[cfg(feature = "collector")]
    #[cfg_attr(docsrs, doc(cfg(feature = "collector")))]
    pub fn set_event_filter(&self, collector: EventFilter) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetEventFilter(collector));
    }

    /// Sets a new filter for a message collector.
    #[inline]
    #[cfg(feature = "collector")]
    #[cfg_attr(docsrs, doc(cfg(feature = "collector")))]
    pub fn set_message_filter(&self, collector: MessageFilter) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetMessageFilter(collector));
    }

    /// Sets a new filter for a reaction collector.
    #[cfg(feature = "collector")]
    #[cfg_attr(docsrs, doc(cfg(feature = "collector")))]
    pub fn set_reaction_filter(&self, collector: ReactionFilter) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetReactionFilter(collector));
    }

    /// Sets a new filter for a component interaction collector.
    #[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
    #[cfg_attr(docsrs, doc(cfg(all(feature = "unstable_discord_api", feature = "collector"))))]
    pub fn set_component_interaction_filter(&self, collector: ComponentInteractionFilter) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.send_to_shard(ShardRunnerMessage::SetComponentInteractionFilter(collector));
    }
}

impl AsRef<ShardMessenger> for ShardMessenger {
    fn as_ref(&self) -> &ShardMessenger {
        self
    }
}
