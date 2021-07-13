use std::{borrow::Cow, sync::Arc};

use async_tungstenite::tungstenite::{
    self,
    error::Error as TungsteniteError,
    protocol::frame::CloseFrame,
};
use futures::channel::mpsc::{self, UnboundedReceiver as Receiver, UnboundedSender as Sender};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument, trace, warn};
use typemap_rev::TypeMap;

use super::event::{ClientEvent, ShardStageUpdateEvent};
use super::{ShardClientMessage, ShardId, ShardManagerMessage, ShardRunnerMessage};
#[cfg(feature = "voice")]
use crate::client::bridge::voice::VoiceGatewayManager;
use crate::client::dispatch::{dispatch, DispatchEvent};
use crate::client::{EventHandler, RawEventHandler};
#[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
use crate::collector::ComponentInteractionFilter;
#[cfg(feature = "collector")]
use crate::collector::{EventFilter, LazyArc, LazyReactionAction, MessageFilter, ReactionFilter};
#[cfg(feature = "framework")]
use crate::framework::Framework;
use crate::gateway::{GatewayError, InterMessage, ReconnectType, Shard, ShardAction};
use crate::internal::prelude::*;
use crate::internal::ws_impl::{ReceiverExt, SenderExt};
use crate::model::event::{Event, GatewayEvent};
#[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
use crate::model::interactions::{Interaction, InteractionType};
use crate::CacheAndHttp;

/// A runner for managing a [`Shard`] and its respective WebSocket client.
pub struct ShardRunner {
    data: Arc<RwLock<TypeMap>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    raw_event_handler: Option<Arc<dyn RawEventHandler>>,
    #[cfg(feature = "framework")]
    framework: Arc<Box<dyn Framework + Send + Sync>>,
    manager_tx: Sender<ShardManagerMessage>,
    // channel to receive messages from the shard manager and dispatches
    runner_rx: Receiver<InterMessage>,
    // channel to send messages to the shard runner from the shard manager
    runner_tx: Sender<InterMessage>,
    pub(crate) shard: Shard,
    #[cfg(feature = "voice")]
    voice_manager: Option<Arc<dyn VoiceGatewayManager + Send + Sync + 'static>>,
    cache_and_http: Arc<CacheAndHttp>,
    #[cfg(feature = "collector")]
    event_filters: Vec<EventFilter>,
    #[cfg(feature = "collector")]
    message_filters: Vec<MessageFilter>,
    #[cfg(feature = "collector")]
    reaction_filters: Vec<ReactionFilter>,
    #[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
    component_interaction_filters: Vec<ComponentInteractionFilter>,
}

impl ShardRunner {
    /// Creates a new runner for a Shard.
    pub fn new(opt: ShardRunnerOptions) -> Self {
        let (tx, rx) = mpsc::unbounded();

        Self {
            runner_rx: rx,
            runner_tx: tx,
            data: opt.data,
            event_handler: opt.event_handler,
            raw_event_handler: opt.raw_event_handler,
            #[cfg(feature = "framework")]
            framework: opt.framework,
            manager_tx: opt.manager_tx,
            shard: opt.shard,
            #[cfg(feature = "voice")]
            voice_manager: opt.voice_manager,
            cache_and_http: opt.cache_and_http,
            #[cfg(feature = "collector")]
            event_filters: Vec::new(),
            #[cfg(feature = "collector")]
            message_filters: Vec::new(),
            #[cfg(feature = "collector")]
            reaction_filters: Vec::new(),
            #[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
            component_interaction_filters: vec![],
        }
    }

    /// Starts the runner's loop to receive events.
    ///
    /// This runs a loop that performs the following in each iteration:
    ///
    /// 1. checks the receiver for [`ShardRunnerMessage`]s, possibly from the
    /// [`ShardManager`], and if there is one, acts on it.
    ///
    /// 2. checks if a heartbeat should be sent to the discord Gateway, and if
    /// so, sends one.
    ///
    /// 3. attempts to retrieve a message from the WebSocket, processing it into
    /// a [`GatewayEvent`]. This will block for 100ms before assuming there is
    /// no message available.
    ///
    /// 4. Checks with the [`Shard`] to determine if the gateway event is
    /// specifying an action to take (e.g. resuming, reconnecting, heartbeating)
    /// and then performs that action, if any.
    ///
    /// 5. Dispatches the event via the Client.
    ///
    /// 6. Go back to 1.
    ///
    /// [`ShardManager`]: super::ShardManager
    #[instrument(skip(self))]
    pub async fn run(&mut self) -> Result<()> {
        info!("[ShardRunner {:?}] Running", self.shard.shard_info());

        loop {
            trace!("[ShardRunner {:?}] loop iteration started.", self.shard.shard_info());
            if !self.recv().await? {
                return Ok(());
            }

            // check heartbeat
            if !self.shard.check_heartbeat().await {
                warn!("[ShardRunner {:?}] Error heartbeating", self.shard.shard_info(),);

                return self.request_restart().await;
            }

            let pre = self.shard.stage();
            let (event, action, successful) = self.recv_event().await?;
            let post = self.shard.stage();

            if post != pre {
                self.update_manager();

                let e = ClientEvent::ShardStageUpdate(ShardStageUpdateEvent {
                    new: post,
                    old: pre,
                    shard_id: ShardId(self.shard.shard_info()[0]),
                });

                self.dispatch(DispatchEvent::Client(e)).await;
            }

            match action {
                Some(ShardAction::Reconnect(ReconnectType::Reidentify)) => {
                    return self.request_restart().await;
                },
                Some(other) => {
                    if let Err(e) = self.action(&other).await {
                        debug!(
                            "[ShardRunner {:?}] Reconnecting due to error performing {:?}: {:?}",
                            self.shard.shard_info(),
                            other,
                            e
                        );
                        match self.shard.reconnection_type() {
                            ReconnectType::Reidentify => return self.request_restart().await,
                            ReconnectType::Resume => {
                                if let Err(why) = self.shard.resume().await {
                                    warn!(
                                        "[ShardRunner {:?}] Resume failed, reidentifying: {:?}",
                                        self.shard.shard_info(),
                                        why
                                    );

                                    return self.request_restart().await;
                                }
                            },
                        };
                    }
                },
                None => {},
            }

            if let Some(event) = event {
                #[cfg(feature = "collector")]
                {
                    self.handle_filters(&event);
                }

                self.dispatch(DispatchEvent::Model(event)).await;
            }

            if !successful && !self.shard.stage().is_connecting() {
                return self.request_restart().await;
            }
            trace!("[ShardRunner {:?}] loop iteration reached the end.", self.shard.shard_info());
        }
    }

    /// Lets filters check the `event` to send them to collectors if the `event`
    /// is accepted by them.
    #[cfg(feature = "collector")]
    fn handle_filters(&mut self, event: &Event) {
        /// Unlike [`Vec`]'s `retain`, allows mutable references in `f`.
        fn retain<T, F>(vec: &mut Vec<T>, mut f: F)
        where
            F: FnMut(&mut T) -> bool,
        {
            let len = vec.len();
            let mut del = 0;
            {
                let v = &mut **vec;

                for i in 0..len {
                    if !f(&mut v[i]) {
                        del += 1;
                    } else if del > 0 {
                        v.swap(i - del, i);
                    }
                }
            }

            if del > 0 {
                vec.truncate(len - del);
            }
        }

        match &event {
            Event::MessageCreate(ref msg_event) => {
                let mut msg = LazyArc::new(&msg_event.message);
                retain(&mut self.message_filters, |f| f.send_message(&mut msg));
            },
            Event::ReactionAdd(ref reaction_event) => {
                let mut reaction = LazyReactionAction::new(&reaction_event.reaction, true);
                retain(&mut self.reaction_filters, |f| f.send_reaction(&mut reaction));
            },
            Event::ReactionRemove(ref reaction_event) => {
                let mut reaction = LazyReactionAction::new(&reaction_event.reaction, false);
                retain(&mut self.reaction_filters, |f| f.send_reaction(&mut reaction));
            },
            #[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
            Event::InteractionCreate(ref interaction_event) => {
                if interaction_event.interaction.kind() == InteractionType::MessageComponent {
                    if let Interaction::MessageComponent(ref interaction) =
                        interaction_event.interaction
                    {
                        let mut interaction = LazyArc::new(interaction);
                        retain(&mut self.component_interaction_filters, |f| {
                            f.send_interaction(&mut interaction)
                        });
                    }
                }
            },
            _ => {},
        }

        let mut event = LazyArc::new(event);
        retain(&mut self.event_filters, |f| f.send_event(&mut event));
    }

    /// Clones the internal copy of the Sender to the shard runner.
    pub(super) fn runner_tx(&self) -> Sender<InterMessage> {
        self.runner_tx.clone()
    }

    /// Takes an action that a [`Shard`] has determined should happen and then
    /// does it.
    ///
    /// For example, if the shard says that an Identify message needs to be
    /// sent, this will do that.
    ///
    /// # Errors
    ///
    /// Returns
    #[instrument(skip(self, action))]
    async fn action(&mut self, action: &ShardAction) -> Result<()> {
        match *action {
            ShardAction::Reconnect(ReconnectType::Reidentify) => self.request_restart().await,
            ShardAction::Reconnect(ReconnectType::Resume) => self.shard.resume().await,
            ShardAction::Heartbeat => self.shard.heartbeat().await,
            ShardAction::Identify => self.shard.identify().await,
        }
    }

    // Checks if the ID received to shutdown is equivalent to the ID of the
    // shard this runner is responsible. If so, it shuts down the WebSocket
    // client.
    //
    // Returns whether the WebSocket client is still active.
    //
    // If true, the WebSocket client was _not_ shutdown. If false, it was.
    #[instrument(skip(self))]
    async fn checked_shutdown(&mut self, id: ShardId, close_code: u16) -> bool {
        // First verify the ID so we know for certain this runner is
        // to shutdown.
        if id.0 != self.shard.shard_info()[0] {
            // Not meant for this runner for some reason, don't
            // shutdown.
            return true;
        }

        // Send a Close Frame to Discord, which allows a bot to "log off"
        #[allow(clippy::let_underscore_must_use)]
        let _ = self
            .shard
            .client
            .close(Some(CloseFrame {
                code: close_code.into(),
                reason: Cow::from(""),
            }))
            .await;

        // In return, we wait for either a Close Frame response, or an error, after which this WS is deemed
        // disconnected from Discord.
        loop {
            match self.shard.client.next().await {
                Some(Ok(tungstenite::Message::Close(_))) => break,
                Some(Err(_)) => {
                    warn!(
                        "[ShardRunner {:?}] Received an error awaiting close frame",
                        self.shard.shard_info(),
                    );
                    break;
                },
                _ => continue,
            }
        }

        // Inform the manager that shutdown for this shard has finished.
        if let Err(why) = self.manager_tx.unbounded_send(ShardManagerMessage::ShutdownFinished(id))
        {
            warn!(
                "[ShardRunner {:?}] Could not send ShutdownFinished: {:#?}",
                self.shard.shard_info(),
                why,
            );
        }
        false
    }

    #[inline]
    #[instrument(skip(self, event))]
    async fn dispatch(&self, event: DispatchEvent) {
        dispatch(
            event,
            #[cfg(feature = "framework")]
            &self.framework,
            &self.data,
            &self.event_handler,
            &self.raw_event_handler,
            &self.runner_tx,
            self.shard.shard_info()[0],
            Arc::clone(&self.cache_and_http),
        )
        .await;
    }

    // Handles a received value over the shard runner rx channel.
    //
    // Returns a boolean on whether the shard runner can continue.
    //
    // This always returns true, except in the case that the shard manager asked
    // the runner to shutdown.
    #[instrument(skip(self))]
    async fn handle_rx_value(&mut self, value: InterMessage) -> bool {
        match value {
            InterMessage::Client(value) => match *value {
                ShardClientMessage::Manager(ShardManagerMessage::Restart(id)) => {
                    self.checked_shutdown(id, 4000).await
                },
                ShardClientMessage::Manager(ShardManagerMessage::Shutdown(id, code)) => {
                    self.checked_shutdown(id, code).await
                },
                ShardClientMessage::Manager(ShardManagerMessage::ShutdownAll) => {
                    // This variant should never be received.
                    warn!("[ShardRunner {:?}] Received a ShutdownAll?", self.shard.shard_info(),);

                    true
                },
                ShardClientMessage::Manager(ShardManagerMessage::ShardUpdate {
                    ..
                }) => {
                    // nb: not sent here

                    true
                },
                ShardClientMessage::Manager(ShardManagerMessage::ShutdownInitiated) => {
                    // nb: not sent here

                    true
                },
                ShardClientMessage::Manager(ShardManagerMessage::ShutdownFinished(_)) => {
                    // nb: not sent here

                    true
                },
                ShardClientMessage::Manager(ShardManagerMessage::ShardDisallowedGatewayIntents)
                | ShardClientMessage::Manager(ShardManagerMessage::ShardInvalidAuthentication)
                | ShardClientMessage::Manager(ShardManagerMessage::ShardInvalidGatewayIntents) => {
                    // These variants should never be received.
                    warn!("[ShardRunner {:?}] Received a ShardError?", self.shard.shard_info(),);

                    true
                },
                ShardClientMessage::Runner(ShardRunnerMessage::ChunkGuild {
                    guild_id,
                    limit,
                    filter,
                    nonce,
                }) => {
                    self.shard.chunk_guild(guild_id, limit, filter, nonce.as_deref()).await.is_ok()
                },
                ShardClientMessage::Runner(ShardRunnerMessage::Close(code, reason)) => {
                    let reason = reason.unwrap_or_else(String::new);
                    let close = CloseFrame {
                        code: code.into(),
                        reason: Cow::from(reason),
                    };
                    self.shard.client.close(Some(close)).await.is_ok()
                },
                ShardClientMessage::Runner(ShardRunnerMessage::Message(msg)) => {
                    self.shard.client.send(msg).await.is_ok()
                },
                ShardClientMessage::Runner(ShardRunnerMessage::SetActivity(activity)) => {
                    // To avoid a clone of `activity`, we do a little bit of
                    // trickery here:
                    //
                    // First, we obtain a reference to the current presence of
                    // the shard, and create a new presence tuple of the new
                    // activity we received over the channel as well as the
                    // online status that the shard already had.
                    //
                    // We then (attempt to) send the websocket message with the
                    // status update, expressively returning:
                    //
                    // - whether the message successfully sent
                    // - the original activity we received over the channel
                    self.shard.set_activity(activity);

                    self.shard.update_presence().await.is_ok()
                },
                ShardClientMessage::Runner(ShardRunnerMessage::SetPresence(status, activity)) => {
                    self.shard.set_presence(status, activity);

                    self.shard.update_presence().await.is_ok()
                },
                ShardClientMessage::Runner(ShardRunnerMessage::SetStatus(status)) => {
                    self.shard.set_status(status);

                    self.shard.update_presence().await.is_ok()
                },
                #[cfg(feature = "collector")]
                ShardClientMessage::Runner(ShardRunnerMessage::SetEventFilter(collector)) => {
                    self.event_filters.push(collector);

                    true
                },
                #[cfg(feature = "collector")]
                ShardClientMessage::Runner(ShardRunnerMessage::SetMessageFilter(collector)) => {
                    self.message_filters.push(collector);

                    true
                },
                #[cfg(feature = "collector")]
                ShardClientMessage::Runner(ShardRunnerMessage::SetReactionFilter(collector)) => {
                    self.reaction_filters.push(collector);

                    true
                },
                #[cfg(all(feature = "unstable_discord_api", feature = "collector"))]
                ShardClientMessage::Runner(ShardRunnerMessage::SetComponentInteractionFilter(
                    collector,
                )) => {
                    self.component_interaction_filters.push(collector);

                    true
                },
            },
            InterMessage::Json(value) => {
                // Value must be forwarded over the websocket
                self.shard.client.send_json(&value).await.is_ok()
            },
        }
    }

    #[cfg(feature = "voice")]
    #[instrument(skip(self))]
    async fn handle_voice_event(&self, event: &Event) {
        if let Some(voice_manager) = &self.voice_manager {
            match *event {
                Event::Ready(_) => {
                    voice_manager
                        .register_shard(self.shard.shard_info()[0], self.runner_tx.clone())
                        .await;
                },
                Event::VoiceServerUpdate(ref event) => {
                    if let Some(guild_id) = event.guild_id {
                        voice_manager.server_update(guild_id, &event.endpoint, &event.token).await;
                    }
                },
                Event::VoiceStateUpdate(ref event) => {
                    if let Some(guild_id) = event.guild_id {
                        voice_manager.state_update(guild_id, &event.voice_state).await;
                    }
                },
                _ => {},
            }
        }
    }

    // Receives values over the internal shard runner rx channel and handles
    // them.
    //
    // This will loop over values until there is no longer one.
    //
    // Requests a restart if the sending half of the channel disconnects. This
    // should _never_ happen, as the sending half is kept on the runner.

    // Returns whether the shard runner is in a state that can continue.
    #[instrument(skip(self))]
    async fn recv(&mut self) -> Result<bool> {
        loop {
            match self.runner_rx.try_next() {
                Ok(Some(value)) => {
                    if !self.handle_rx_value(value).await {
                        return Ok(false);
                    }
                },
                Ok(None) => {
                    warn!(
                        "[ShardRunner {:?}] Sending half DC; restarting",
                        self.shard.shard_info(),
                    );

                    #[allow(clippy::let_underscore_must_use)]
                    let _ = self.request_restart().await;

                    return Ok(false);
                },
                Err(_) => break,
            }
        }

        // There are no longer any values available.

        Ok(true)
    }

    /// Returns a received event, as well as whether reading the potentially
    /// present event was successful.
    #[instrument(skip(self))]
    async fn recv_event(&mut self) -> Result<(Option<Event>, Option<ShardAction>, bool)> {
        let gw_event = match self.shard.client.recv_json().await {
            Ok(Some(value)) => GatewayEvent::deserialize(value).map(Some).map_err(From::from),
            Ok(None) => Ok(None),
            Err(Error::Tungstenite(TungsteniteError::Io(_))) => {
                debug!("Attempting to auto-reconnect");

                match self.shard.reconnection_type() {
                    ReconnectType::Reidentify => return Ok((None, None, false)),
                    ReconnectType::Resume => {
                        if let Err(why) = self.shard.resume().await {
                            warn!("Failed to resume: {:?}", why);

                            return Ok((None, None, false));
                        }
                    },
                }

                return Ok((None, None, true));
            },
            Err(why) => Err(why),
        };

        let event = match gw_event {
            Ok(Some(event)) => Ok(event),
            Ok(None) => return Ok((None, None, true)),
            Err(why) => Err(why),
        };

        let action = match self.shard.handle_event(&event) {
            Ok(Some(action)) => Some(action),
            Ok(None) => None,
            Err(why) => {
                error!("Shard handler received err: {:?}", why);

                match why {
                    Error::Gateway(GatewayError::InvalidAuthentication) => {
                        if self
                            .manager_tx
                            .unbounded_send(ShardManagerMessage::ShardInvalidAuthentication)
                            .is_err()
                        {
                            panic!(
                                "Failed sending InvalidAuthentication error to the shard manager."
                            );
                        }

                        return Err(why);
                    },
                    Error::Gateway(GatewayError::InvalidGatewayIntents) => {
                        if self
                            .manager_tx
                            .unbounded_send(ShardManagerMessage::ShardInvalidGatewayIntents)
                            .is_err()
                        {
                            panic!(
                                "Failed sending InvalidGatewayIntents error to the shard manager."
                            );
                        }

                        return Err(why);
                    },
                    Error::Gateway(GatewayError::DisallowedGatewayIntents) => {
                        if self
                            .manager_tx
                            .unbounded_send(ShardManagerMessage::ShardDisallowedGatewayIntents)
                            .is_err()
                        {
                            panic!("Failed sending DisallowedGatewayIntents error to the shard manager.");
                        }

                        return Err(why);
                    },
                    _ => return Ok((None, None, true)),
                }
            },
        };

        if let Ok(GatewayEvent::HeartbeatAck) = event {
            self.update_manager();
        }

        #[cfg(feature = "voice")]
        {
            if let Ok(GatewayEvent::Dispatch(_, ref event)) = event {
                self.handle_voice_event(&event).await;
            }
        }

        let event = match event {
            Ok(GatewayEvent::Dispatch(_, event)) => Some(event),
            _ => None,
        };

        Ok((event, action, true))
    }

    #[instrument(skip(self))]
    async fn request_restart(&mut self) -> Result<()> {
        self.update_manager();

        debug!("[ShardRunner {:?}] Requesting restart", self.shard.shard_info(),);
        let shard_id = ShardId(self.shard.shard_info()[0]);
        let msg = ShardManagerMessage::Restart(shard_id);

        if let Err(error) = self.manager_tx.unbounded_send(msg) {
            warn!("Error sending request restart: {:?}", error);
        }

        #[cfg(feature = "voice")]
        if let Some(voice_manager) = &self.voice_manager {
            voice_manager.deregister_shard(shard_id.0).await;
        }

        Ok(())
    }

    #[instrument(skip(self))]
    fn update_manager(&self) {
        #[allow(clippy::let_underscore_must_use)]
        let _ = self.manager_tx.unbounded_send(ShardManagerMessage::ShardUpdate {
            id: ShardId(self.shard.shard_info()[0]),
            latency: self.shard.latency(),
            stage: self.shard.stage(),
        });
    }
}

/// Options to be passed to [`ShardRunner::new`].
pub struct ShardRunnerOptions {
    pub data: Arc<RwLock<TypeMap>>,
    pub event_handler: Option<Arc<dyn EventHandler>>,
    pub raw_event_handler: Option<Arc<dyn RawEventHandler>>,
    #[cfg(feature = "framework")]
    pub framework: Arc<Box<dyn Framework + Send + Sync>>,
    pub manager_tx: Sender<ShardManagerMessage>,
    pub shard: Shard,
    #[cfg(feature = "voice")]
    pub voice_manager: Option<Arc<dyn VoiceGatewayManager + Send + Sync>>,
    pub cache_and_http: Arc<CacheAndHttp>,
}
