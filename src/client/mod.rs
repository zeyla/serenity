//! The Client contains information about a single bot's token, as well
//! as event handlers. Dispatching events to configured handlers and starting
//! the shards' connections are handled directly via the client. In addition,
//! the `http` module and `Cache` are also automatically handled by the
//! Client module for you.
//!
//! A [`Context`] is provided for every handler.
//!
//! The `http` module is the lower-level method of interacting with the Discord
//! REST API. Realistically, there should be little reason to use this yourself,
//! as the Context will do this for you. A possible use case of using the `http`
//! module is if you do not have a Cache, for purposes such as low memory
//! requirements.
//!
//! Click [here][Client examples] for an example on how to use a `Client`.
//!
//! [Client examples]: Client#examples

pub mod bridge;

mod context;
#[cfg(feature = "gateway")]
mod dispatch;
mod error;
#[cfg(feature = "gateway")]
mod event_handler;
#[cfg(feature = "gateway")]
mod extras;

use std::time::Duration;
use std::{
    boxed::Box,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context as FutContext, Poll},
};

use futures::future::BoxFuture;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, instrument};
use typemap_rev::{TypeMap, TypeMapKey};

#[cfg(feature = "gateway")]
use self::bridge::gateway::{
    GatewayIntents,
    ShardManager,
    ShardManagerError,
    ShardManagerMonitor,
    ShardManagerOptions,
};
#[cfg(feature = "voice")]
use self::bridge::voice::VoiceGatewayManager;
pub use self::{context::Context, error::Error as ClientError};
#[cfg(feature = "gateway")]
pub use self::{
    event_handler::{EventHandler, RawEventHandler},
    extras::Extras,
};
#[cfg(feature = "gateway")]
use super::gateway::GatewayError;
#[cfg(feature = "cache")]
pub use crate::cache::Cache;
#[cfg(feature = "framework")]
use crate::framework::Framework;
use crate::http::Http;
use crate::internal::prelude::*;
#[cfg(feature = "unstable_discord_api")]
use crate::model::id::ApplicationId;
use crate::model::id::UserId;
pub use crate::CacheAndHttp;

/// A builder implementing [`Future`] building a [`Client`] to interact with Discord.
#[cfg(feature = "gateway")]
pub struct ClientBuilder<'a> {
    // FIXME: Remove this allow attribute once `application_id` is no longer feature-gated
    // under `unstable_discord_api`.
    #[allow(dead_code)]
    token: Option<String>,
    data: Option<TypeMap>,
    http: Option<Http>,
    fut: Option<BoxFuture<'a, Result<Client>>>,
    intents: GatewayIntents,
    #[cfg(feature = "unstable_discord_api")]
    application_id: Option<ApplicationId>,
    timeout: Option<Duration>,
    http_request_timeout: Option<Duration>,
    http_connect_timeout: Option<Duration>,
    #[cfg(feature = "framework")]
    framework: Option<Arc<Box<dyn Framework + Send + Sync + 'static>>>,
    #[cfg(feature = "voice")]
    voice_manager: Option<Arc<dyn VoiceGatewayManager + Send + Sync + 'static>>,
    event_handler: Option<Arc<dyn EventHandler>>,
    raw_event_handler: Option<Arc<dyn RawEventHandler>>,
}

#[cfg(feature = "gateway")]
impl<'a> ClientBuilder<'a> {
    fn _new() -> Self {
        Self {
            token: None,
            data: Some(TypeMap::new()),
            http: None,
            fut: None,
            intents: GatewayIntents::non_privileged(),
            #[cfg(feature = "unstable_discord_api")]
            application_id: None,
            timeout: None,
            http_request_timeout: None,
            http_connect_timeout: None,
            #[cfg(feature = "framework")]
            framework: None,
            #[cfg(feature = "voice")]
            voice_manager: None,
            event_handler: None,
            raw_event_handler: None,
        }
    }

    /// Construct a new builder to call methods on for the client construction.
    /// The `token` will automatically be prefixed "Bot " if not already.
    ///
    /// **Panic**:
    /// If you have enabled the `framework`-feature (on by default), you must specify
    /// a framework via the [`Self::framework`] or [`Self::framework_arc`] method,
    /// otherwise awaiting the builder will cause a panic.
    pub fn new(token: impl AsRef<str>) -> Self {
        Self::_new().token(token)
    }

    /// Construct a new builder with a [`Http`] instance to calls methods on
    /// for the client construction.
    ///
    /// **Panic**:
    /// If you have enabled the `framework`-feature (on by default), you must specify
    /// a framework via the [`Self::framework`] or [`Self::framework_arc`] method,
    /// otherwise awaiting the builder will cause a panic.
    ///
    /// [`Http`]: crate::http::Http
    pub fn new_with_http(http: Http) -> Self {
        let mut c = Self::_new();
        c.http = Some(http);
        c
    }

    /// Sets a token for the bot. If the token is not prefixed "Bot ",
    /// this method will automatically do so.
    pub fn token(mut self, token: impl AsRef<str>) -> Self {
        let token = token.as_ref().trim();

        let token =
            if token.starts_with("Bot ") { token.to_string() } else { format!("Bot {}", token) };

        self.http = Some(Http::new_with_timeouts(
            self.http_request_timeout,
            self.http_connect_timeout,
            &token,
        ));

        self.token = Some(token.clone());

        self
    }

    /// Sets the application id.
    #[cfg(feature = "unstable_discord_api")]
    pub fn application_id(mut self, application_id: u64) -> Self {
        self.application_id = Some(ApplicationId(application_id));

        self.http = Some(Http::new_with_token_application_id(
            &self.token.clone().unwrap(),
            application_id,
            self.http_request_timeout,
            self.http_connect_timeout,
        ));

        self
    }

    /// Sets the entire [`TypeMap`] that will be available in [`Context`]s.
    /// A [`TypeMap`] must not be constructed manually: [`Self::type_map_insert`]
    /// can be used to insert one type at a time.
    pub fn type_map(mut self, type_map: TypeMap) -> Self {
        self.data = Some(type_map);

        self
    }

    /// Insert a single `value` into the internal [`TypeMap`] that will
    /// be available in [`Context::data`].
    /// This method can be called multiple times in order to populate the
    /// [`TypeMap`] with `value`s.
    pub fn type_map_insert<T: TypeMapKey>(mut self, value: T::Value) -> Self {
        if let Some(ref mut data) = self.data {
            data.insert::<T>(value);
        } else {
            let mut type_map = TypeMap::new();
            type_map.insert::<T>(value);

            self.data = Some(type_map);
        }

        self
    }

    /// Sets how long - if wanted to begin with - a cache update shall
    /// be attempted for. After the `timeout` ran out, the update will be
    /// skipped.
    ///
    /// By default, a cache update will never timeout and potentially
    /// cause a deadlock.
    /// A timeout however, will invalidate the cache.
    #[cfg(feature = "cache")]
    pub fn cache_update_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);

        self
    }

    /// Sets the command framework to be used. It will receive messages sent
    /// over the gateway and then consider - based on its settings - whether to
    /// dispatch a command.
    ///
    /// *Info*:
    /// If a reference to the framework is required for manual dispatch,
    /// use the [`Self::framework_arc`]-method instead.
    #[cfg(feature = "framework")]
    pub fn framework<F>(mut self, framework: F) -> Self
    where
        F: Framework + Send + Sync + 'static,
    {
        self.framework = Some(Arc::new(Box::new(framework)));

        self
    }

    /// This method allows to pass an [`Arc`]'ed `framework` - this step is
    /// done for you in the [`Self::framework`]-method, if you don't need the
    /// extra control.
    /// You can provide a clone and keep the original to manually dispatch.
    #[cfg(feature = "framework")]
    pub fn framework_arc(
        mut self,
        framework: Arc<Box<dyn Framework + Send + Sync + 'static>>,
    ) -> Self {
        self.framework = Some(framework);

        self
    }

    /// Sets the voice gateway handler to be used. It will receive voice events sent
    /// over the gateway and then consider - based on its settings - whether to
    /// dispatch a command.
    ///
    /// *Info*:
    /// If a reference to the voice_manager is required for manual dispatch,
    /// use the [`Self::voice_manager_arc`]-method instead.
    #[cfg(feature = "voice")]
    pub fn voice_manager<V>(mut self, voice_manager: V) -> Self
    where
        V: VoiceGatewayManager + Send + Sync + 'static,
    {
        self.voice_manager = Some(Arc::new(voice_manager));

        self
    }

    /// This method allows to pass an [`Arc`]'ed `voice_manager` - this step is
    /// done for you in the [`voice_manager`]-method, if you don't need the
    /// extra control.
    /// You can provide a clone and keep the original to manually dispatch.
    ///
    /// [`voice_manager`]: #method.voice_manager
    #[cfg(feature = "voice")]
    pub fn voice_manager_arc(
        mut self,
        voice_manager: Arc<dyn VoiceGatewayManager + Send + Sync + 'static>,
    ) -> Self {
        self.voice_manager = Some(voice_manager);

        self
    }

    /// Sets all intents directly, replacing already set intents.
    /// Intents are a bitflag, you can combine them by performing the
    /// `|`-operator.
    ///
    /// # What are Intents
    ///
    /// A [gateway intent] sets the types of gateway events
    /// (e.g. member joins, guild integrations, guild emoji updates, ...) the
    /// bot shall receive. Carefully picking the needed intents greatly helps
    /// the bot to scale, as less intents will result in less events to be
    /// received hence less processed by the bot.
    ///
    /// # Privileged Intents
    ///
    /// The intents [`GatewayIntents::GUILD_PRESENCES`] and [`GatewayIntents::GUILD_MEMBERS`]
    /// are *privileged*.
    /// [Privileged intents] need to be enabled in the *developer portal*.
    /// Once the bot is in 100 guilds or more, [the bot must be verified] in
    /// order to use privileged intents.
    ///
    /// [gateway intent]: https://discord.com/developers/docs/topics/gateway#privileged-intents
    /// [Privileged intents]: https://discord.com/developers/docs/topics/gateway#privileged-intents
    /// [the bot must be verified]: https://support.discord.com/hc/en-us/articles/360040720412-Bot-Verification-and-Data-Whitelisting
    /// [`GatewayIntents::GUILD_PRESENCES`]: crate::client::bridge::gateway::GatewayIntents::GUILD_PRESENCES
    /// [`GatewayIntents::GUILD_MEMBERS`]: crate::client::bridge::gateway::GatewayIntents::GUILD_MEMBERS
    pub fn intents(mut self, intents: GatewayIntents) -> Self {
        self.intents = intents;

        self
    }

    /// Sets an event handler with multiple methods for each possible event.
    pub fn event_handler<H: EventHandler + 'static>(mut self, event_handler: H) -> Self {
        self.event_handler = Some(Arc::new(event_handler));

        self
    }

    /// Sets an event handler with a single method where all received gateway
    /// events will be dispatched.
    pub fn raw_event_handler<H: RawEventHandler + 'static>(mut self, raw_event_handler: H) -> Self {
        self.raw_event_handler = Some(Arc::new(raw_event_handler));

        self
    }

    /// Sets how long the http client should wait for the entire response from the Discord API.
    /// By default, a http request will never timeout, it could create a deadlock sometimes.
    ///
    /// **Note**:
    /// Setting http request timeout will override any custom http instance configured on the client.
    pub fn http_request_timeout(mut self, timeout: Duration) -> Self {
        self.http_request_timeout = Some(timeout);
        let old_token = match self.http {
            Some(old_http) => old_http.token.to_owned(),
            None => String::new(),
        };
        #[cfg(feature = "unstable_discord_api")]
        {
            self.http = Some(Http::new_with_token_application_id(
                &old_token,
                *self.application_id.unwrap_or_default().as_u64(),
                self.http_request_timeout,
                self.http_connect_timeout,
            ));
        }

        #[cfg(not(feature = "unstable_discord_api"))]
        {
            self.http = Some(Http::new_with_timeouts(
                self.http_request_timeout,
                self.http_connect_timeout,
                &old_token,
            ));
        }
        self
    }

    /// Sets how long the http client should wait for only the connect phase to discord API.
    /// By default, an http request will never timeout, it could create a deadlock sometimes.
    ///
    /// *Info*:
    /// Setting http request timeout will override any custom http instance configured on the client.
    pub fn http_connect_timeout(mut self, timeout: Duration) -> Self {
        self.http_connect_timeout = Some(timeout);
        let old_token = match self.http {
            Some(old_http) => old_http.token.to_owned(),
            None => String::new(),
        };

        #[cfg(feature = "unstable_discord_api")]
        {
            self.http = Some(Http::new_with_token_application_id(
                &old_token,
                *self.application_id.unwrap_or_default().as_u64(),
                self.http_request_timeout,
                self.http_connect_timeout,
            ));
        }

        #[cfg(not(feature = "unstable_discord_api"))]
        {
            self.http = Some(Http::new_with_timeouts(
                self.http_request_timeout,
                self.http_connect_timeout,
                &old_token,
            ));
        }

        self
    }
}

#[cfg(feature = "gateway")]
impl<'a> Future for ClientBuilder<'a> {
    type Output = Result<Client>;

    #[allow(clippy::unwrap_used)] // Allowing unwrap because all should be Some() by this point
    #[instrument(skip(self))]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut FutContext<'_>) -> Poll<Self::Output> {
        if self.fut.is_none() {
            let data = Arc::new(RwLock::new(self.data.take().unwrap()));
            #[cfg(feature = "framework")]
            let framework = self.framework.take()
                .expect("The `framework`-feature is enabled (it's on by default), but no framework was provided.\n\
                If you don't want to use the command framework, disable default features and specify all features you want to use.");
            let event_handler = self.event_handler.take();
            let raw_event_handler = self.raw_event_handler.take();
            let intents = self.intents;
            let http = Arc::new(self.http.take().unwrap());

            #[cfg(feature = "unstable_discord_api")]
            if http.application_id == 0 {
                panic!("Please provide an Application Id in order to use interactions features.");
            }

            #[cfg(feature = "voice")]
            let voice_manager = self.voice_manager.take();

            let cache_and_http = Arc::new(CacheAndHttp {
                #[cfg(feature = "cache")]
                cache: Arc::new(Cache::default()),
                #[cfg(feature = "cache")]
                update_cache_timeout: self.timeout.take(),
                http: Arc::clone(&http),
            });

            self.fut = Some(Box::pin(async move {
                let url = Arc::new(Mutex::new(http.get_gateway().await?.url));

                let (shard_manager, shard_manager_worker) = {
                    ShardManager::new(ShardManagerOptions {
                        data: &data,
                        event_handler: &event_handler,
                        raw_event_handler: &raw_event_handler,
                        #[cfg(feature = "framework")]
                        framework: &framework,
                        shard_index: 0,
                        shard_init: 0,
                        shard_total: 0,
                        #[cfg(feature = "voice")]
                        voice_manager: &voice_manager,
                        ws_url: &url,
                        cache_and_http: &cache_and_http,
                        intents,
                    })
                    .await
                };

                Ok(Client {
                    ws_uri: url,
                    data,
                    shard_manager,
                    shard_manager_worker,
                    #[cfg(feature = "voice")]
                    voice_manager,
                    cache_and_http,
                })
            }))
        }

        self.fut.as_mut().unwrap().as_mut().poll(ctx)
    }
}

/// The Client is the way to be able to start sending authenticated requests
/// over the REST API, as well as initializing a WebSocket connection through
/// [`Shard`]s. Refer to the [documentation on using sharding][sharding docs]
/// for more information.
///
/// # Event Handlers
///
/// Event handlers can be configured. For example, the event handler
/// [`EventHandler::message`] will be dispatched to whenever a
/// [`Event::MessageCreate`] is received over the connection.
///
/// Note that you do not need to manually handle events, as they are handled
/// internally and then dispatched to your event handlers.
///
/// # Examples
///
/// Creating a Client instance and adding a handler on every message
/// receive, acting as a "ping-pong" bot is simple:
///
/// ```no_run
/// use serenity::model::prelude::*;
/// use serenity::prelude::*;
/// use serenity::Client;
///
/// struct Handler;
///
/// #[serenity::async_trait]
/// impl EventHandler for Handler {
///     async fn message(&self, context: Context, msg: Message) {
///         if msg.content == "!ping" {
///             let _ = msg.channel_id.say(&context, "Pong!");
///         }
///     }
/// }
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let mut client = Client::builder("my token here").event_handler(Handler).await?;
///
/// client.start().await?;
/// #   Ok(())
/// # }
/// ```
///
/// [`Shard`]: crate::gateway::Shard
/// [`Event::MessageCreate`]: crate::model::event::Event::MessageCreate
/// [sharding docs]: crate::gateway#sharding
#[cfg(feature = "gateway")]
pub struct Client {
    /// A TypeMap which requires types to be Send + Sync. This is a map that
    /// can be safely shared across contexts.
    ///
    /// The purpose of the data field is to be accessible and persistent across
    /// contexts; that is, data can be modified by one context, and will persist
    /// through the future and be accessible through other contexts. This is
    /// useful for anything that should "live" through the program: counters,
    /// database connections, custom user caches, etc.
    ///
    /// In the meaning of a context, this data can be accessed through
    /// [`Context::data`].
    ///
    /// # Examples
    ///
    /// Create a `MessageEventCounter` to track the following events:
    ///
    /// - [`Event::MessageCreate`]
    /// - [`Event::MessageDelete`]
    /// - [`Event::MessageDeleteBulk`]
    /// - [`Event::MessageUpdate`]
    ///
    /// ```rust,ignore
    /// use serenity::prelude::*;
    /// use serenity::model::prelude::*;
    /// use std::collections::HashMap;
    /// use std::env;
    ///
    /// struct MessageEventCounter;
    ///
    /// impl TypeMapKey for MessageEventCounter {
    ///     type Value = HashMap<String, u64>;
    /// }
    ///
    /// async fn reg<S: Into<String>>(ctx: Context, name: S) {
    ///     let mut data = ctx.data.write().await;
    ///     let counter = data.get_mut::<MessageEventCounter>().unwrap();
    ///     let entry = counter.entry(name.into()).or_insert(0);
    ///     *entry += 1;
    /// }
    ///
    /// struct Handler;
    ///
    /// #[serenity::async_trait]
    /// impl EventHandler for Handler {
    ///     async fn message(&self, ctx: Context, _: Message) {
    ///         reg(ctx, "MessageCreate").await
    ///     }
    ///     async fn message_delete(&self, ctx: Context, _: ChannelId, _: MessageId) {
    ///         reg(ctx, "MessageDelete").await
    ///     }
    ///     async fn message_delete_bulk(&self, ctx: Context, _: ChannelId, _: Vec<MessageId>) {
    ///         reg(ctx, "MessageDeleteBulk").await
    ///     }
    ///
    ///     #[cfg(feature = "cache")]
    ///     async fn message_update(&self, ctx: Context, _old: Option<Message>, _new: Option<Message>, _: MessageUpdateEvent) {
    ///         reg(ctx, "MessageUpdate").await
    ///     }
    ///
    ///     #[cfg(not(feature = "cache"))]
    ///     async fn message_update(&self, ctx: Context, _new_data: MessageUpdateEvent) {
    ///         reg(ctx, "MessageUpdate").await
    ///     }
    /// }
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    /// {
    ///     let mut data = client.data.write().await;
    ///     data.insert::<MessageEventCounter>(HashMap::default());
    /// }
    ///
    /// client.start().await?;
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// Refer to [example 05] for an example on using the [`Self::data`] field.
    ///
    /// [`Event::MessageCreate`]: crate::model::event::Event::MessageCreate
    /// [`Event::MessageDelete`]: crate::model::event::Event::MessageDelete
    /// [`Event::MessageDeleteBulk`]: crate::model::event::Event::MessageDeleteBulk
    /// [`Event::MessageUpdate`]: crate::model::event::Event::MessageUpdate
    /// [example 05]: https://github.com/serenity-rs/serenity/tree/current/examples/e05_command_framework
    pub data: Arc<RwLock<TypeMap>>,
    /// A HashMap of all shards instantiated by the Client.
    ///
    /// The key is the shard ID and the value is the shard itself.
    ///
    /// # Examples
    ///
    /// If you call [`client.start_shard(3, 5)`][`Client::start_shard`], this
    /// HashMap will only ever contain a single key of `3`, as that's the only
    /// Shard the client is responsible for.
    ///
    /// If you call [`client.start_shards(10)`][`Client::start_shards`], this
    /// HashMap will contain keys 0 through 9, one for each shard handled by the
    /// client.
    ///
    /// Printing the number of shards currently instantiated by the client every
    /// 5 seconds:
    ///
    /// ```rust,no_run
    /// # use serenity::client::{Client, EventHandler};
    /// # use std::time::Duration;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// struct Handler;
    ///
    /// impl EventHandler for Handler { }
    ///
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// let shard_manager = client.shard_manager.clone();
    ///
    /// tokio::spawn(async move {
    ///     loop {
    ///         let sm = shard_manager.lock().await;
    ///         let count = sm.shards_instantiated().await.len();
    ///         println!("Shard count instantiated: {}", count);
    ///
    ///         tokio::time::sleep(Duration::from_millis(5000)).await;
    ///     }
    /// });
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// Shutting down all connections after one minute of operation:
    ///
    /// ```rust,no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use serenity::client::{Client, EventHandler};
    /// use std::time::Duration;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler { }
    ///
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// // Create a clone of the `Arc` containing the shard manager.
    /// let shard_manager = client.shard_manager.clone();
    ///
    /// // Create a thread which will sleep for 60 seconds and then have the
    /// // shard manager shutdown.
    /// tokio::spawn(async move {
    ///     tokio::time::sleep(Duration::from_secs(60));
    ///
    ///     shard_manager.lock().await.shutdown_all().await;
    ///
    ///     println!("Shutdown shard manager!");
    /// });
    ///
    /// println!("Client shutdown: {:?}", client.start().await);
    /// #     Ok(())
    /// # }
    /// ```
    pub shard_manager: Arc<Mutex<ShardManager>>,
    shard_manager_worker: ShardManagerMonitor,
    /// The voice manager for the client.
    ///
    /// This is an ergonomic structure for interfacing over shards' voice
    /// connections.
    #[cfg(feature = "voice")]
    pub voice_manager: Option<Arc<dyn VoiceGatewayManager + Send + Sync + 'static>>,
    /// URI that the client's shards will use to connect to the gateway.
    ///
    /// This is likely not important for production usage and is, at best, used
    /// for debugging.
    ///
    /// This is wrapped in an `Arc<Mutex<T>>` so all shards will have an updated
    /// value available.
    pub ws_uri: Arc<Mutex<String>>,
    /// A container for an optional cache and HTTP client.
    /// It also contains the cache update timeout.
    pub cache_and_http: Arc<CacheAndHttp>,
}

impl Client {
    pub fn builder<'a>(token: impl AsRef<str>) -> ClientBuilder<'a> {
        ClientBuilder::new(token)
    }

    /// Establish the connection and start listening for events.
    ///
    /// This will start receiving events in a loop and start dispatching the
    /// events to your registered handlers.
    ///
    /// Note that this should be used only for users and for bots which are in
    /// less than 2500 guilds. If you have a reason for sharding and/or are in
    /// more than 2500 guilds, use one of these depending on your use case:
    ///
    /// Refer to the [Gateway documentation][gateway docs] for more information
    /// on effectively using sharding.
    ///
    /// # Examples
    ///
    /// Starting a Client with only 1 shard, out of 1 total:
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start().await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// [gateway docs]: crate::gateway#sharding
    #[instrument(skip(self))]
    pub async fn start(&mut self) -> Result<()> {
        self.start_connection([0, 0, 1]).await
    }

    /// Establish the connection(s) and start listening for events.
    ///
    /// This will start receiving events in a loop and start dispatching the
    /// events to your registered handlers.
    ///
    /// This will retrieve an automatically determined number of shards to use
    /// from the API - determined by Discord - and then open a number of shards
    /// equivalent to that amount.
    ///
    /// Refer to the [Gateway documentation][gateway docs] for more information
    /// on effectively using sharding.
    ///
    /// # Examples
    ///
    /// Start as many shards as needed using autosharding:
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start_autosharded().await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError::Shutdown`] when all shards have shutdown due to
    /// an error.
    ///
    /// [gateway docs]: crate::gateway#sharding
    #[instrument(skip(self))]
    pub async fn start_autosharded(&mut self) -> Result<()> {
        let (x, y) = {
            let res = self.cache_and_http.http.get_bot_gateway().await?;

            (res.shards as u64 - 1, res.shards as u64)
        };

        self.start_connection([0, x, y]).await
    }

    /// Establish a sharded connection and start listening for events.
    ///
    /// This will start receiving events and dispatch them to your registered
    /// handlers.
    ///
    /// This will create a single shard by ID. If using one shard per process,
    /// you will need to start other processes with the other shard IDs in some
    /// way.
    ///
    /// Refer to the [Gateway documentation][gateway docs] for more information
    /// on effectively using sharding.
    ///
    /// # Examples
    ///
    /// Start shard 3 of 5:
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start_shard(3, 5).await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// Start shard 0 of 1 (you may also be interested in [`Self::start`] or
    /// [`Self::start_autosharded`]):
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start_shard(0, 1).await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError::Shutdown`] when all shards have shutdown due to
    /// an error.
    ///
    /// [gateway docs]: crate::gateway#sharding
    #[instrument(skip(self))]
    pub async fn start_shard(&mut self, shard: u64, shards: u64) -> Result<()> {
        self.start_connection([shard, shard, shards]).await
    }

    /// Establish sharded connections and start listening for events.
    ///
    /// This will start receiving events and dispatch them to your registered
    /// handlers.
    ///
    /// This will create and handle all shards within this single process. If
    /// you only need to start a single shard within the process, or a range of
    /// shards, use [`Self::start_shard`] or [`Self::start_shard_range`], respectively.
    ///
    /// Refer to the [Gateway documentation][gateway docs] for more information
    /// on effectively using sharding.
    ///
    /// # Examples
    ///
    /// Start all of 8 shards:
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start_shards(8).await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError::Shutdown`] when all shards have shutdown due to
    /// an error.
    ///
    /// [Gateway docs]: crate::gateway#sharding
    #[instrument(skip(self))]
    pub async fn start_shards(&mut self, total_shards: u64) -> Result<()> {
        self.start_connection([0, total_shards - 1, total_shards]).await
    }

    /// Establish a range of sharded connections and start listening for events.
    ///
    /// This will start receiving events and dispatch them to your registered
    /// handlers.
    ///
    /// This will create and handle all shards within a given range within this
    /// single process. If you only need to start a single shard within the
    /// process, or all shards within the process, use [`Self::start_shard`] or
    /// [`Self::start_shards`], respectively.
    ///
    /// Refer to the [Gateway documentation][gateway docs] for more
    /// information on effectively using sharding.
    ///
    /// # Examples
    ///
    /// For a bot using a total of 10 shards, initialize shards 4 through 7:
    ///
    /// ```rust,no_run
    /// # use std::error::Error;
    /// # use serenity::prelude::EventHandler;
    /// use serenity::Client;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {}
    ///
    /// # async fn run() -> Result<(), Box<dyn Error>> {
    /// let token = std::env::var("DISCORD_TOKEN")?;
    /// let mut client = Client::builder(&token).event_handler(Handler).await?;
    ///
    /// if let Err(why) = client.start_shard_range([4, 7], 10).await {
    ///     println!("Err with client: {:?}", why);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError::Shutdown`] when all shards have shutdown due to
    /// an error.
    ///
    /// [Gateway docs]: crate::gateway#sharding
    #[instrument(skip(self))]
    pub async fn start_shard_range(&mut self, range: [u64; 2], total_shards: u64) -> Result<()> {
        self.start_connection([range[0], range[1], total_shards]).await
    }

    /// Shard data layout is:
    /// 0: first shard number to initialize
    /// 1: shard number to initialize up to and including
    /// 2: total number of shards the bot is sharding for
    ///
    /// Not all shards need to be initialized in this process.
    ///
    /// # Errors
    ///
    /// Returns a [`ClientError::Shutdown`] when all shards have shutdown due to
    /// an error.
    #[instrument(skip(self))]
    async fn start_connection(&mut self, shard_data: [u64; 3]) -> Result<()> {
        #[cfg(feature = "voice")]
        if let Some(voice_manager) = &self.voice_manager {
            let user = self.cache_and_http.http.get_current_user().await?;

            voice_manager.initialise(shard_data[2], user.id).await;
        }

        {
            let mut manager = self.shard_manager.lock().await;

            let init = shard_data[1] - shard_data[0] + 1;

            manager.set_shards(shard_data[0], init, shard_data[2]).await;

            debug!("Initializing shard info: {} - {}/{}", shard_data[0], init, shard_data[2],);

            if let Err(why) = manager.initialize() {
                error!("Failed to boot a shard: {:?}", why);
                info!("Shutting down all shards");

                manager.shutdown_all().await;

                return Err(Error::Client(ClientError::ShardBootFailure));
            }
        }

        if let Err(why) = self.shard_manager_worker.run().await {
            let err = match why {
                ShardManagerError::DisallowedGatewayIntents => {
                    GatewayError::DisallowedGatewayIntents
                },
                ShardManagerError::InvalidGatewayIntents => GatewayError::InvalidGatewayIntents,
                ShardManagerError::InvalidToken => GatewayError::InvalidAuthentication,
            };
            return Err(Error::Gateway(err));
        }

        Ok(())
    }
}

/// Validates that a token is likely in a valid format.
///
/// This performs the following checks on a given token:
///
/// - At least one character long;
/// - Contains 3 parts (split by the period char `'.'`);
/// - The second part of the token is at least 6 characters long;
/// - The token does not contain any whitespace prior to or after the token.
///
/// # Examples
///
/// Validate that a token is valid and that a number of invalid tokens are
/// actually invalid:
///
/// ```rust,no_run
/// use serenity::client::validate_token;
///
/// // ensure a valid token is in fact valid:
/// assert!(validate_token("Mjg4NzYwMjQxMzYzODc3ODg4.C_ikow.j3VupLBuE1QWZng3TMGH0z_UAwg").is_ok());
///
/// // helpful to prevent typos
/// assert!(validate_token("Njg4NzYwMjQxMzYzODc3ODg4.C_ikow.j3VupLBuE1QWZng3TMGH0z_UAwg").is_err());
/// ```
///
/// # Errors
///
/// Returns a [`ClientError::InvalidToken`] when one of the above checks fail.
/// The type of failure is not specified.
pub fn validate_token(token: impl AsRef<str>) -> Result<()> {
    if parse_token(token.as_ref()).is_some() {
        Ok(())
    } else {
        Err(Error::Client(ClientError::InvalidToken))
    }
}

/// Part of the data contained within a Discord bot token. Returned by [`parse_token`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenComponents {
    pub bot_user_id: UserId,
    pub creation_time: chrono::NaiveDateTime,
}

/// Verifies that the token adheres to the Discord token format and extracts the bot user ID and the
/// token generation timestamp
pub fn parse_token(token: impl AsRef<str>) -> Option<TokenComponents> {
    // The token consists of three base64-encoded parts
    let parts: Vec<&str> = token.as_ref().split('.').collect();
    let base64_config = base64::Config::new(base64::CharacterSet::UrlSafe, true);

    // First part must be a base64-encoded stringified user ID
    let user_id = base64::decode_config(parts.get(0)?, base64_config).ok()?;
    let user_id = UserId(std::str::from_utf8(&user_id).ok()?.parse().ok()?);

    // Second part must be a base64-encoded token generation timestamp
    let timestamp_base64 = parts.get(1)?;
    // The base64-encoded timestamp must be at least 6 characters
    if timestamp_base64.len() < 6 {
        return None;
    }
    let timestamp_bytes = base64::decode_config(timestamp_base64, base64_config).ok()?;
    let mut timestamp = 0;
    for byte in timestamp_bytes {
        timestamp *= 256;
        timestamp += byte as u64;
    }
    // Some timestamps are based on the Discord epoch. Convert to Unix epoch
    if timestamp < 1293840000 {
        timestamp += 1293840000;
    }
    let timestamp = chrono::NaiveDateTime::from_timestamp_opt(timestamp as i64, 0)?;

    // Third part is a base64-encoded HMAC that's not interesting on its own
    let _ = base64::decode_config(parts.get(2)?, base64_config).ok()?;

    Some(TokenComponents {
        bot_user_id: user_id,
        creation_time: timestamp,
    })
}
