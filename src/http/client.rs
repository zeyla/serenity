#![allow(clippy::missing_errors_doc)]
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{Context as FutContext, Poll},
};

use futures::future::BoxFuture;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::{
    header::{HeaderMap as Headers, HeaderValue, CONTENT_TYPE},
    StatusCode,
    Url,
};
use reqwest::{Client, ClientBuilder, Response as ReqwestResponse};
use serde::de::DeserializeOwned;
#[cfg(feature = "simd-json")]
use simd_json::ValueAccess;
use tracing::{debug, instrument, trace};

use super::{
    multipart::Multipart,
    ratelimiting::{RatelimitedRequest, Ratelimiter},
    request::Request,
    routing::RouteInfo,
    typing::Typing,
    AttachmentType,
    GuildPagination,
    HttpError,
};
use crate::constants;
use crate::internal::prelude::*;
use crate::json::{from_number, from_value, json, to_value, to_vec};
#[cfg(feature = "unstable_discord_api")]
use crate::model::interactions::application_command::{
    ApplicationCommand,
    ApplicationCommandPermission,
};
use crate::model::prelude::*;

/// A builder implementing [`Future`] building a [`Http`] client to perform
/// requests to Discord's HTTP API. If you do not need to use a proxy or do not
/// need to disable the rate limiter, you can use [`Http::new`] or
/// [`Http::new_with_token`] instead.
///
/// ## Example
///
/// Create an instance of [`Http`] with a proxy and rate limiter disabled
///
/// ```rust
/// # use serenity::http::HttpBuilder;
/// # async fn run() {
/// let http = HttpBuilder::new("token")
///     .proxy("http://127.0.0.1:3000")
///     .expect("Invalid proxy URL")
///     .ratelimiter_disabled(true)
///     .await
///     .expect("Error creating Http");
/// # }
/// ```
pub struct HttpBuilder<'a> {
    client: Option<Arc<Client>>,
    ratelimiter: Option<Ratelimiter>,
    ratelimiter_disabled: Option<bool>,
    token: Option<String>,
    proxy: Option<Url>,
    fut: Option<BoxFuture<'a, Result<Http>>>,
    #[cfg(feature = "unstable_discord_api")]
    application_id: Option<u64>,
}

impl<'a> HttpBuilder<'a> {
    fn _new() -> Self {
        Self {
            client: None,
            ratelimiter: None,
            ratelimiter_disabled: Some(false),
            token: None,
            proxy: None,
            fut: None,
            #[cfg(feature = "unstable_discord_api")]
            application_id: None,
        }
    }

    /// Construct a new builder to call methods on for the HTTP construction.
    /// The `token` will automatically be prefixed "Bot " if not already.
    pub fn new(token: impl AsRef<str>) -> Self {
        Self::_new().token(token)
    }

    /// Sets the application_id to use interactions.
    #[cfg(feature = "unstable_discord_api")]
    pub fn application_id(mut self, application_id: u64) -> Self {
        self.application_id = Some(application_id);

        self
    }

    /// Sets a token for the bot. If the token is not prefixed "Bot ", this
    /// method will automatically do so.
    pub fn token(mut self, token: impl AsRef<str>) -> Self {
        let token = token.as_ref().trim();

        let token =
            if token.starts_with("Bot ") { token.to_string() } else { format!("Bot {}", token) };

        self.token = Some(token);

        self
    }

    /// Sets the [`reqwest::Client`]. If one isn't provided, a default one will
    /// be used.
    pub fn client(mut self, client: Arc<Client>) -> Self {
        self.client = Some(client);

        self
    }

    /// Sets the ratelimiter to be used. If one isn't provided, a default one
    /// will be used.
    pub fn ratelimiter(mut self, ratelimiter: Ratelimiter) -> Self {
        self.ratelimiter = Some(ratelimiter);

        self
    }

    /// Sets whether or not the ratelimiter is disabled. By default if this this
    /// not used, it is enabled. In most cases, this should be used in
    /// conjunction with [`Self::proxy`].
    ///
    /// **Note**: You should **not** disable the ratelimiter unless you have
    /// another form of rate limiting. Disabling the ratelimiter has the main
    /// purpose of delegating rate limiting to an API proxy via [`Self::proxy`]
    /// instead of the current process.
    pub fn ratelimiter_disabled(mut self, ratelimiter_disabled: bool) -> Self {
        self.ratelimiter_disabled = Some(ratelimiter_disabled);

        self
    }

    /// Sets the proxy that Discord HTTP API requests will be passed to. This is
    /// mainly intended for something like [`twilight-http-proxy`] where
    /// multiple processes can make API requests while sharing a single
    /// ratelimiter.
    ///
    /// The proxy should be in the form of the protocol and hostname, e.g.
    /// `http://127.0.0.1:3000` or `http://myproxy.example`
    ///
    /// This will simply send HTTP API requests to the proxy instead of Discord
    /// API to allow the proxy to intercept, rate limit, and forward requests.
    /// This is different than a native proxy's behavior where it will tunnel
    /// requests that use TLS via [`HTTP CONNECT`] method (e.g. using
    /// [`reqwest::Proxy`]).
    ///
    /// [`twilight-http-proxy`]: https://github.com/twilight-rs/http-proxy
    /// [`HTTP CONNECT`]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Methods/CONNECT
    pub fn proxy(mut self, proxy: impl Into<String>) -> Result<Self> {
        let proxy = Url::from_str(&proxy.into()).map_err(HttpError::Url)?;
        self.proxy = Some(proxy);

        Ok(self)
    }
}

impl<'a> Future for HttpBuilder<'a> {
    type Output = Result<Http>;

    #[allow(clippy::unwrap_used)]
    #[instrument(skip(self))]
    fn poll(mut self: Pin<&mut Self>, ctx: &mut FutContext<'_>) -> Poll<Self::Output> {
        if self.fut.is_none() {
            let token = self.token.take().unwrap();

            #[cfg(feature = "unstable_discord_api")]
            let application_id = self
                .application_id
                .expect("Expected application Id in order to use interacions features");

            let client = self.client.take().unwrap_or_else(|| {
                let builder = configure_client_backend(Client::builder());
                Arc::new(builder.build().expect("Cannot build reqwest::Client"))
            });

            let ratelimiter = self.ratelimiter.take().unwrap_or_else(|| {
                let client = Arc::clone(&client);
                Ratelimiter::new(client, token.to_string())
            });

            let ratelimiter_disabled = self.ratelimiter_disabled.take().unwrap();
            let proxy = self.proxy.take();

            self.fut = Some(Box::pin(async move {
                Ok(Http {
                    client,
                    ratelimiter,
                    ratelimiter_disabled,
                    proxy,
                    token,
                    #[cfg(feature = "unstable_discord_api")]
                    application_id,
                })
            }))
        }

        self.fut.as_mut().unwrap().as_mut().poll(ctx)
    }
}

fn reason_into_header(reason: &str) -> Headers {
    let mut headers = Headers::new();
    headers.insert(
        "X-Audit-Log-Reason",
        HeaderValue::from_str(&Cow::from(utf8_percent_encode(reason, NON_ALPHANUMERIC)))
            .expect("Invalid header value even after percent encode"),
    );
    headers
}

/// **Note**: For all member functions that return a [`Result`], the
/// Error kind will be either [`Error::Http`] or [`Error::Json`].
///
/// [`Error::Http`]: crate::error::Error::Http
/// [`Error::Json`]: crate::error::Error::Json
pub struct Http {
    pub(crate) client: Arc<Client>,
    pub ratelimiter: Ratelimiter,
    pub ratelimiter_disabled: bool,
    pub proxy: Option<Url>,
    pub token: String,
    #[cfg(feature = "unstable_discord_api")]
    pub application_id: u64,
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("client", &self.client)
            .field("ratelimiter", &self.ratelimiter)
            .field("ratelimiter_disabled", &self.ratelimiter_disabled)
            .field("proxy", &self.proxy)
            .finish()
    }
}

impl Http {
    pub fn new(client: Arc<Client>, token: &str) -> Self {
        let client2 = Arc::clone(&client);

        Http {
            client,
            ratelimiter: Ratelimiter::new(client2, token.to_string()),
            ratelimiter_disabled: false,
            proxy: None,
            token: token.to_string(),
            #[cfg(feature = "unstable_discord_api")]
            application_id: 0,
        }
    }

    #[cfg(feature = "unstable_discord_api")]
    pub fn new_with_application_id(application_id: u64) -> Self {
        let builder = configure_client_backend(Client::builder());
        let built = builder.build().expect("Cannot build reqwest::Client");

        let mut data = Self::new(Arc::new(built), "");

        data.application_id = application_id;

        data
    }

    pub fn new_with_token(token: &str) -> Self {
        let builder = configure_client_backend(Client::builder());
        let built = builder.build().expect("Cannot build reqwest::Client");

        let trimmed = token.trim();
        let token = if trimmed.starts_with("Bot ") || trimmed.starts_with("Bearer ") {
            token.to_string()
        } else {
            format!("Bot {}", token)
        };

        Self::new(Arc::new(built), &token)
    }

    #[cfg(feature = "unstable_discord_api")]
    pub fn new_with_token_application_id(token: &str, application_id: u64) -> Self {
        let mut base = Self::new_with_token(token);

        base.application_id = application_id;

        base
    }

    /// Adds a single [`Role`] to a [`Member`] in a [`Guild`].
    ///
    /// **Note**: Requires the [Manage Roles] permission and respect of role
    /// hierarchy.
    ///
    /// [Manage Roles]: Permissions::MANAGE_ROLES
    pub async fn add_member_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::AddMemberRole {
                guild_id,
                role_id,
                user_id,
            },
        })
        .await
    }

    /// Bans a [`User`] from a [`Guild`], removing their messages sent in the last
    /// X number of days.
    ///
    /// Passing a `delete_message_days` of `0` is equivalent to not removing any
    /// messages. Up to `7` days' worth of messages may be deleted.
    ///
    /// **Note**: Requires that you have the [Ban Members] permission.
    ///
    /// [Ban Members]: Permissions::BAN_MEMBERS
    pub async fn ban_user(
        &self,
        guild_id: u64,
        user_id: u64,
        delete_message_days: u8,
        reason: &str,
    ) -> Result<()> {
        // Audit log reason and normal ban reason are mutually exclusive so we don't need the audit
        // log reason parameter here

        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GuildBanUser {
                delete_message_days: Some(delete_message_days),
                reason: Some(&utf8_percent_encode(reason, NON_ALPHANUMERIC).to_string()),
                guild_id,
                user_id,
            },
        })
        .await
    }

    /// Broadcasts that the current user is typing in the given [`Channel`].
    ///
    /// This lasts for about 10 seconds, and will then need to be renewed to
    /// indicate that the current user is still typing.
    ///
    /// This should rarely be used for bots, although it is a good indicator that a
    /// long-running command is still being processed.
    pub async fn broadcast_typing(&self, channel_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::BroadcastTyping {
                channel_id,
            },
        })
        .await
    }

    /// Creates a [`GuildChannel`] in the [`Guild`] given its Id.
    ///
    /// Refer to the Discord's [docs] for information on what fields this requires.
    ///
    /// **Note**: Requires the [Manage Channels] permission.
    ///
    /// [docs]: https://discord.com/developers/docs/resources/guild#create-guild-channel
    /// [Manage Channels]: Permissions::MANAGE_CHANNELS
    pub async fn create_channel(
        &self,
        guild_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<GuildChannel> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::CreateChannel {
                guild_id,
            },
        })
        .await
    }

    /// Creates a stage instance.
    pub async fn create_stage_instance(&self, map: &Value) -> Result<StageInstance> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateStageInstance,
        })
        .await
    }

    /// Creates a public thread channel in the [`GuildChannel`] given its Id,
    /// with a base message Id.
    pub async fn create_public_thread(
        &self,
        channel_id: u64,
        message_id: u64,
        map: &JsonMap,
    ) -> Result<GuildChannel> {
        let body = serde_json::to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::CreatePublicThread {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Creates a private thread channel in the [`GuildChannel`] given its Id.
    pub async fn create_private_thread(
        &self,
        channel_id: u64,
        map: &JsonMap,
    ) -> Result<GuildChannel> {
        let body = serde_json::to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::CreatePrivateThread {
                channel_id,
            },
        })
        .await
    }

    /// Creates an emoji in the given [`Guild`] with the given data.
    ///
    /// View the source code for [`Guild::create_emoji`] method to see what
    /// fields this requires.
    ///
    /// **Note**: Requires the [Manage Emojis and Stickers] permission.
    ///
    /// [Manage Emojis and Stickers]: Permissions::MANAGE_EMOJIS_AND_STICKERS
    pub async fn create_emoji(
        &self,
        guild_id: u64,
        map: &Value,
        audit_log_reason: Option<&str>,
    ) -> Result<Emoji> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::CreateEmoji {
                guild_id,
            },
        })
        .await
    }

    /// Create a follow-up message for an Interaction.
    ///
    /// Functions the same as [`Self::execute_webhook`]
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_followup_message(
        &self,
        interaction_token: &str,
        map: &Value,
    ) -> Result<Message> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateFollowupMessage {
                application_id: self.application_id,
                interaction_token,
            },
        })
        .await
    }

    /// Create a follow-up message with attachments for an Interaction.
    ///
    /// Functions the same as [`Self::execute_webhook`]
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_followup_message_with_files(
        &self,
        interaction_token: &str,
        map: &Value,
        files: impl IntoIterator<Item = AttachmentType<'_>>,
    ) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: files.into_iter().map(Into::into).collect(),
                payload_json: Some(map.clone()),
                fields: vec![],
            }),
            headers: None,
            route: RouteInfo::CreateFollowupMessage {
                application_id: self.application_id,
                interaction_token,
            },
        })
        .await
    }

    /// Creates a new global command.
    ///
    /// New global commands will be available in all guilds after 1 hour.
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// **Note**:
    /// Creating a command with the same name as an existing command for your
    /// application will overwrite the old command.
    ///
    /// [docs]: https://discord.com/developers/docs/interactions/slash-commands#create-global-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_global_application_command(
        &self,
        map: &Value,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateGlobalApplicationCommand {
                application_id: self.application_id,
            },
        })
        .await
    }

    /// Creates new global application commands.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_global_application_commands(
        &self,
        map: &Value,
    ) -> Result<Vec<ApplicationCommand>> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateGlobalApplicationCommands {
                application_id: self.application_id,
            },
        })
        .await
    }

    /// Creates new guild application commands.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_guild_application_commands(
        &self,
        guild_id: u64,
        map: &Value,
    ) -> Result<Vec<ApplicationCommand>> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateGuildApplicationCommands {
                application_id: self.application_id,
                guild_id,
            },
        })
        .await
    }

    /// Creates a guild with the data provided.
    ///
    /// Only a [`PartialGuild`] will be immediately returned, and a full [`Guild`]
    /// will be received over a [`Shard`], if at least one is running.
    ///
    /// **Note**: This endpoint is currently limited to 10 active guilds. The
    /// limits are raised for whitelisted [GameBridge] applications. See the
    /// [documentation on this endpoint] for more info.
    ///
    /// # Examples
    ///
    /// Create a guild called `"test"` in the [US West region]:
    ///
    /// ```rust,no_run
    /// use serenity::json::json;
    /// use serenity::http::Http;
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #    let http = Http::default();
    /// let map = json!({
    ///     "name": "test",
    ///     "region": "us-west",
    /// });
    ///
    /// let _result = http.create_guild(&map).await?;
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// [`Shard`]: crate::gateway::Shard
    /// [GameBridge]: https://discord.com/developers/docs/topics/gamebridge
    /// [US West Region]: Region::UsWest
    /// [documentation on this endpoint]:
    /// https://discord.com/developers/docs/resources/guild#create-guild
    /// [whitelist]: https://discord.com/developers/docs/resources/guild#create-guild
    pub async fn create_guild(&self, map: &Value) -> Result<PartialGuild> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateGuild,
        })
        .await
    }

    /// Creates a new guild command.
    ///
    /// New guild commands will be available in the guild immediately.
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/interactions/slash-commands#create-guild-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_guild_application_command(
        &self,
        guild_id: u64,
        map: &Value,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateGuildApplicationCommand {
                application_id: self.application_id,
                guild_id,
            },
        })
        .await
    }

    /// Creates an [`Integration`] for a [`Guild`].
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// **Note**: Requires the [Manage Guild] permission.
    ///
    /// [Manage Guild]: Permissions::MANAGE_GUILD
    /// [docs]: https://discord.com/developers/docs/resources/guild#create-guild-integration
    pub async fn create_guild_integration(
        &self,
        guild_id: u64,
        integration_id: u64,
        map: &Value,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::CreateGuildIntegration {
                guild_id,
                integration_id,
            },
        })
        .await
    }

    /// Creates a response to an [`Interaction`] from the gateway.
    ///
    /// Refer to Discord's [docs] for the object it takes.
    ///
    /// [docs]: https://discord.com/developers/docs/interactions/slash-commands#interaction-interaction-response
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn create_interaction_response(
        &self,
        interaction_id: u64,
        interaction_token: &str,
        map: &Value,
    ) -> Result<()> {
        self.wind(204, Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateInteractionResponse {
                interaction_id,
                interaction_token,
            },
        })
        .await
    }

    /// Creates a [`RichInvite`] for the given [channel][`GuildChannel`].
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// All fields are optional.
    ///
    /// **Note**: Requires the [Create Invite] permission.
    ///
    /// [Create Invite]: Permissions::CREATE_INVITE
    /// [docs]: https://discord.com/developers/docs/resources/channel#create-channel-invite
    pub async fn create_invite(
        &self,
        channel_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<RichInvite> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::CreateInvite {
                channel_id,
            },
        })
        .await
    }

    /// Creates a permission override for a member or a role in a channel.
    pub async fn create_permission(
        &self,
        channel_id: u64,
        target_id: u64,
        map: &Value,
    ) -> Result<()> {
        let body = to_vec(map)?;

        self.wind(204, Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::CreatePermission {
                channel_id,
                target_id,
            },
        })
        .await
    }

    /// Creates a private channel with a user.
    pub async fn create_private_channel(&self, map: &Value) -> Result<PrivateChannel> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::CreatePrivateChannel,
        })
        .await
    }

    /// Reacts to a message.
    pub async fn create_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        reaction_type: &ReactionType,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::CreateReaction {
                // Escape emojis like '#️⃣' that contain a hash
                reaction: &reaction_type.as_data().replace('#', "%23"),
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Creates a role.
    pub async fn create_role(
        &self,
        guild_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<Role> {
        let body = to_vec(map)?;
        let mut value = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: audit_log_reason.map(reason_into_header),
                route: RouteInfo::CreateRole {
                    guild_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value(value).map_err(From::from)
    }

    /// Creates a sticker.
    ///
    /// **Note**: Requires the [Manage Emojis and Stickers] permission.
    ///
    /// [Manage Emojis and Stickers]: Permissions::MANAGE_EMOJIS_AND_STICKERS
    pub async fn create_sticker<'a, T>(
        &self,
        guild_id: u64,
        map: JsonMap,
        file: T,
    ) -> Result<Sticker>
    where
        T: Into<AttachmentType<'a>>,
    {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: vec![file.into()],
                fields: map
                    .into_iter()
                    .map(|(name, value)| {
                        (
                            name.into(),
                            value
                                .as_str()
                                .expect("Create_sticker map must be strings")
                                .to_string()
                                .into(),
                        )
                    })
                    .collect(),
                payload_json: None,
            }),
            headers: None,
            route: RouteInfo::CreateSticker {
                guild_id,
            },
        })
        .await
    }

    /// Creates a webhook for the given [channel][`GuildChannel`]'s Id, passing in
    /// the given data.
    ///
    /// This method requires authentication.
    ///
    /// The Value is a map with the values of:
    ///
    /// - **avatar**: base64-encoded 128x128 image for the webhook's default avatar
    ///   (_optional_);
    /// - **name**: the name of the webhook, limited to between 2 and 100 characters
    ///   long.
    ///
    /// # Examples
    ///
    /// Creating a webhook named `test`:
    ///
    /// ```rust,no_run
    /// use serenity::json::json;
    /// use serenity::http::Http;
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #    let http = Http::default();
    /// let channel_id = 81384788765712384;
    /// let map = json!({"name": "test"});
    ///
    /// let webhook = http.create_webhook(channel_id, &map, None).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn create_webhook(
        &self,
        channel_id: u64,
        map: &Value,
        audit_log_reason: Option<&str>,
    ) -> Result<Webhook> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::CreateWebhook {
                channel_id,
            },
        })
        .await
    }

    /// Deletes a private channel or a channel in a guild.
    pub async fn delete_channel(&self, channel_id: u64) -> Result<Channel> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteChannel {
                channel_id,
            },
        })
        .await
    }

    /// Deletes a stage instance.
    pub async fn delete_stage_instance(&self, channel_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteStageInstance {
                channel_id,
            },
        })
        .await
    }

    /// Deletes an emoji from a server.
    pub async fn delete_emoji(&self, guild_id: u64, emoji_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteEmoji {
                guild_id,
                emoji_id,
            },
        })
        .await
    }

    /// Deletes a follow-up message for an interaction.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn delete_followup_message(
        &self,
        interaction_token: &str,
        message_id: u64,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteFollowupMessage {
                application_id: self.application_id,
                interaction_token,
                message_id,
            },
        })
        .await
    }

    /// Deletes a global command.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn delete_global_application_command(&self, command_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteGlobalApplicationCommand {
                application_id: self.application_id,
                command_id,
            },
        })
        .await
    }

    /// Deletes a guild, only if connected account owns it.
    pub async fn delete_guild(&self, guild_id: u64) -> Result<PartialGuild> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteGuild {
                guild_id,
            },
        })
        .await
    }

    /// Deletes a guild command.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn delete_guild_application_command(
        &self,
        guild_id: u64,
        command_id: u64,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteGuildApplicationCommand {
                application_id: self.application_id,
                guild_id,
                command_id,
            },
        })
        .await
    }

    /// Removes an integration from a guild.
    pub async fn delete_guild_integration(&self, guild_id: u64, integration_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteGuildIntegration {
                guild_id,
                integration_id,
            },
        })
        .await
    }

    /// Deletes an invite by code.
    pub async fn delete_invite(&self, code: &str) -> Result<Invite> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteInvite {
                code,
            },
        })
        .await
    }

    /// Deletes a message if created by us or we have
    /// specific permissions.
    pub async fn delete_message(&self, channel_id: u64, message_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Deletes a bunch of messages, only works for bots.
    pub async fn delete_messages(&self, channel_id: u64, map: &Value) -> Result<()> {
        self.wind(204, Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteMessages {
                channel_id,
            },
        })
        .await
    }

    /// Deletes all of the [`Reaction`]s associated with a [`Message`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// use serenity::model::id::{ChannelId, MessageId};
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// # let http = Http::default();
    /// let channel_id = ChannelId(7);
    /// let message_id = MessageId(8);
    ///
    /// http.delete_message_reactions(channel_id.0, message_id.0).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn delete_message_reactions(&self, channel_id: u64, message_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteMessageReactions {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Deletes all the reactions for a given emoji on a message.
    pub async fn delete_message_reaction_emoji(
        &self,
        channel_id: u64,
        message_id: u64,
        reaction_type: &ReactionType,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteMessageReactionEmoji {
                reaction: &reaction_type.as_data(),
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Deletes the initial interaction response.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn delete_original_interaction_response(
        &self,
        interaction_token: &str,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteOriginalInteractionResponse {
                application_id: self.application_id,
                interaction_token,
            },
        })
        .await
    }

    /// Deletes a permission override from a role or a member in a channel.
    pub async fn delete_permission(&self, channel_id: u64, target_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeletePermission {
                channel_id,
                target_id,
            },
        })
        .await
    }

    /// Deletes a reaction from a message if owned by us or
    /// we have specific permissions.
    pub async fn delete_reaction(
        &self,
        channel_id: u64,
        message_id: u64,
        user_id: Option<u64>,
        reaction_type: &ReactionType,
    ) -> Result<()> {
        let user = user_id.map(|uid| uid.to_string()).unwrap_or_else(|| "@me".to_string());

        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteReaction {
                // Escape emojis like '#️⃣' that contain a hash
                reaction: &reaction_type.as_data().replace('#', "%23"),
                user: &user,
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Deletes a role from a server. Can't remove the default everyone role.
    pub async fn delete_role(&self, guild_id: u64, role_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteRole {
                guild_id,
                role_id,
            },
        })
        .await
    }

    /// Deletes a sticker from a server.
    ///
    /// **Note**: Requires the [Manage Emojis and Stickers] permission.
    ///
    /// [Manage Emojis and Stickers]: Permissions::MANAGE_EMOJIS_AND_STICKERS
    pub async fn delete_sticker(&self, guild_id: u64, sticker_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteSticker {
                guild_id,
                sticker_id,
            },
        })
        .await
    }

    /// Deletes a [`Webhook`] given its Id.
    ///
    /// This method requires authentication, whereas [`Self::delete_webhook_with_token`]
    /// does not.
    ///
    /// # Examples
    ///
    /// Deletes a webhook given its Id:
    ///
    /// ```rust,no_run
    /// use serenity::http::Http;
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// // Due to the `delete_webhook` function requiring you to authenticate, you
    /// // must have set the token first.
    /// let http = Http::default();
    ///
    /// http.delete_webhook(245037420704169985).await?;
    ///       Ok(())
    /// # }
    /// ```
    pub async fn delete_webhook(&self, webhook_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteWebhook {
                webhook_id,
            },
        })
        .await
    }

    /// Deletes a [`Webhook`] given its Id and unique token.
    ///
    /// This method does _not_ require authentication.
    ///
    /// # Examples
    ///
    /// Deletes a webhook given its Id and unique token:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// # let http = Http::default();
    /// let id = 245037420704169985;
    /// let token = "ig5AO-wdVWpCBtUUMxmgsWryqgsW3DChbKYOINftJ4DCrUbnkedoYZD0VOH1QLr-S3sV";
    ///
    /// http.delete_webhook_with_token(id, token).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn delete_webhook_with_token(&self, webhook_id: u64, token: &str) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteWebhookWithToken {
                token,
                webhook_id,
            },
        })
        .await
    }

    /// Changes channel information.
    pub async fn edit_channel(
        &self,
        channel_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<GuildChannel> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::EditChannel {
                channel_id,
            },
        })
        .await
    }

    /// Edits a stage instance.
    pub async fn edit_stage_instance(&self, channel_id: u64, map: &Value) -> Result<StageInstance> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditStageInstance {
                channel_id,
            },
        })
        .await
    }

    /// Changes emoji information.
    pub async fn edit_emoji(
        &self,
        guild_id: u64,
        emoji_id: u64,
        map: &Value,
        audit_log_reason: Option<&str>,
    ) -> Result<Emoji> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::EditEmoji {
                guild_id,
                emoji_id,
            },
        })
        .await
    }

    /// Edits a follow-up message for an interaction.
    ///
    /// Refer to Discord's [docs] for Edit Webhook Message for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/resources/webhook#edit-webhook-message
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_followup_message(
        &self,
        interaction_token: &str,
        message_id: u64,
        map: &Value,
    ) -> Result<Message> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditFollowupMessage {
                application_id: self.application_id,
                interaction_token,
                message_id,
            },
        })
        .await
    }

    /// Edits a follow-up message and its attachments for an interaction.
    ///
    /// Refer to Discord's [docs] for Edit Webhook Message for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/resources/webhook#edit-webhook-message
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_followup_message_and_attachments(
        &self,
        interaction_token: &str,
        message_id: u64,
        map: &Value,
        new_attachments: impl IntoIterator<Item = AttachmentType<'_>>,
    ) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: new_attachments.into_iter().map(Into::into).collect(),
                payload_json: Some(map.clone()),
                fields: vec![],
            }),
            headers: None,
            route: RouteInfo::EditFollowupMessage {
                application_id: self.application_id,
                interaction_token,
                message_id,
            },
        })
        .await
    }

    /// Edits a global command.
    ///
    /// Updates will be available in all guilds after 1 hour.
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/interactions/slash-commands#edit-global-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_global_application_command(
        &self,
        command_id: u64,
        map: &Value,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGlobalApplicationCommand {
                application_id: self.application_id,
                command_id,
            },
        })
        .await
    }

    /// Changes guild information.
    pub async fn edit_guild(
        &self,
        guild_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<PartialGuild> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::EditGuild {
                guild_id,
            },
        })
        .await
    }

    /// Edits a guild command.
    ///
    /// Updates for guild commands will be available immediately.
    ///
    /// Refer to Discord's [docs] for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/interactions/slash-commands#edit-guild-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_guild_application_command(
        &self,
        guild_id: u64,
        command_id: u64,
        map: &Value,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildApplicationCommand {
                application_id: self.application_id,
                guild_id,
                command_id,
            },
        })
        .await
    }

    /// Edits a guild command permissions.
    ///
    /// Updates for guild commands will be available immediately.
    ///
    /// Refer to Discord's [documentation] for field information.
    ///
    /// [documentation]: https://discord.com/developers/docs/interactions/slash-commands#edit-guild-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_guild_application_command_permissions(
        &self,
        guild_id: u64,
        command_id: u64,
        map: &Value,
    ) -> Result<ApplicationCommandPermission> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildApplicationCommandPermission {
                application_id: self.application_id,
                guild_id,
                command_id,
            },
        })
        .await
    }

    /// Edits a guild commands permissions.
    ///
    /// Updates for guild commands will be available immediately.
    ///
    /// Refer to Discord's [documentation] for field information.
    ///
    /// [documentation]: https://discord.com/developers/docs/interactions/slash-commands#edit-guild-application-command
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_guild_application_commands_permissions(
        &self,
        guild_id: u64,
        map: &Value,
    ) -> Result<Vec<ApplicationCommandPermission>> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildApplicationCommandsPermissions {
                application_id: self.application_id,
                guild_id,
            },
        })
        .await
    }

    /// Edits the positions of a guild's channels.
    pub async fn edit_guild_channel_positions(&self, guild_id: u64, value: &Value) -> Result<()> {
        let body = to_vec(value)?;

        self.wind(204, Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildChannels {
                guild_id,
            },
        })
        .await
    }

    /// Edits a [`Guild`]'s widget.
    pub async fn edit_guild_widget(&self, guild_id: u64, map: &Value) -> Result<GuildWidget> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildWidget {
                guild_id,
            },
        })
        .await
    }

    /// Edits a guild welcome screen.
    pub async fn edit_guild_welcome_screen(
        &self,
        guild_id: u64,
        map: &Value,
    ) -> Result<GuildWelcomeScreen> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditGuildWelcomeScreen {
                guild_id,
            },
        })
        .await
    }

    /// Does specific actions to a member.
    pub async fn edit_member(
        &self,
        guild_id: u64,
        user_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<Member> {
        let body = to_vec(map)?;

        let mut value = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: audit_log_reason.map(reason_into_header),
                route: RouteInfo::EditMember {
                    guild_id,
                    user_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value::<Member>(value).map_err(From::from)
    }

    /// Edits a message by Id.
    ///
    /// **Note**: Only the author of a message can modify it.
    pub async fn edit_message(
        &self,
        channel_id: u64,
        message_id: u64,
        map: &Value,
    ) -> Result<Message> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Edits a message and its attachments by Id.
    ///
    /// **Note**: Only the author of a message can modify it.
    pub async fn edit_message_and_attachments(
        &self,
        channel_id: u64,
        message_id: u64,
        map: &Value,
        new_attachments: impl IntoIterator<Item = AttachmentType<'_>>,
    ) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: new_attachments.into_iter().map(Into::into).collect(),
                payload_json: Some(map.clone()),
                fields: vec![],
            }),
            headers: None,
            route: RouteInfo::EditMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Crossposts a message by Id.
    ///
    /// **Note**: Only available on announcements channels.
    pub async fn crosspost_message(&self, channel_id: u64, message_id: u64) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::CrosspostMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Edits the current member for the provided [`Guild`] via its Id.
    pub async fn edit_member_me(&self, guild_id: u64, map: &JsonMap) -> Result<Member> {
        let body = serde_json::to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditMemberMe {
                guild_id,
            },
        })
        .await
    }

    /// Edits the current user's nickname for the provided [`Guild`] via its Id.
    ///
    /// Pass [`None`] to reset the nickname.
    pub async fn edit_nickname(&self, guild_id: u64, new_nickname: Option<&str>) -> Result<()> {
        let map = json!({ "nick": new_nickname });
        let body = to_vec(&map)?;

        self.wind(200, Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditMemberMe {
                guild_id,
            },
        })
        .await
    }

    /// Gets the initial interaction response.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_original_interaction_response(
        &self,
        interaction_token: &str,
    ) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetOriginalInteractionResponse {
                application_id: self.application_id,
                interaction_token,
            },
        })
        .await
    }

    /// Edits the initial interaction response.
    ///
    /// Refer to Discord's [docs] for Edit Webhook Message for field information.
    ///
    /// [docs]: https://discord.com/developers/docs/resources/webhook#edit-webhook-message
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn edit_original_interaction_response(
        &self,
        interaction_token: &str,
        map: &Value,
    ) -> Result<Message> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: None,
            route: RouteInfo::EditOriginalInteractionResponse {
                application_id: self.application_id,
                interaction_token,
            },
        })
        .await
    }

    /// Edits the current user's profile settings.
    pub async fn edit_profile(&self, map: &JsonMap) -> Result<CurrentUser> {
        let body = to_vec(map)?;

        let request = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: None,
                route: RouteInfo::EditProfile,
            })
            .await?;

        Ok(request.json::<CurrentUser>().await?)
    }

    /// Changes a role in a guild.
    pub async fn edit_role(
        &self,
        guild_id: u64,
        role_id: u64,
        map: &JsonMap,
        audit_log_reason: Option<&str>,
    ) -> Result<Role> {
        let body = to_vec(&map)?;
        let mut value = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: audit_log_reason.map(reason_into_header),
                route: RouteInfo::EditRole {
                    guild_id,
                    role_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value(value).map_err(From::from)
    }

    /// Changes the position of a role in a guild.
    pub async fn edit_role_position(
        &self,
        guild_id: u64,
        role_id: u64,
        position: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<Vec<Role>> {
        let body = to_vec(&json!([{
            "id": role_id,
            "position": position,
        }]))?;

        let mut value = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: audit_log_reason.map(reason_into_header),
                route: RouteInfo::EditRolePosition {
                    guild_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(array) = value.as_array_mut() {
            for role in array {
                if let Some(map) = role.as_object_mut() {
                    map.insert("guild_id".to_string(), from_number(guild_id));
                }
            }
        }

        from_value(value).map_err(From::from)
    }

    /// Changes a sticker in a guild.
    ///
    /// **Note**: Requires the [Manage Emojis and Stickers] permission.
    ///
    /// [Manage Emojis and Stickers]: Permissions::MANAGE_EMOJIS_AND_STICKERS
    pub async fn edit_sticker(
        &self,
        guild_id: u64,
        sticker_id: u64,
        map: &JsonMap,
    ) -> Result<Sticker> {
        let body = serde_json::to_vec(&map)?;
        let mut value = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: None,
                route: RouteInfo::EditSticker {
                    guild_id,
                    sticker_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value(value).map_err(From::from)
    }

    /// Changes another user's voice state in a stage channel.
    ///
    /// The Value is a map with values of:
    ///
    /// - **channel_id**: ID of the channel the user is currently in
    ///   (**required**)
    /// - **supress**: Bool which toggles user's suppressed state. Setting this
    ///   to `false` will invite the user to speak.
    ///
    /// # Example
    ///
    /// Suppress a user
    ///
    /// ```rust,no_run
    /// use serenity::{
    ///     http::Http,
    ///     json::{json, prelude::*},
    /// };
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let guild_id = 187450744427773963;
    /// let user_id = 150443906511667200;
    /// let value = json!({
    ///     "channel_id": "826929611849334784",
    ///     "suppress": true,
    /// });
    ///
    /// let map = value.as_object().unwrap();
    ///
    /// // Edit state for another user
    /// http.edit_voice_state(guild_id, user_id, &map).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn edit_voice_state(&self, guild_id: u64, user_id: u64, map: &JsonMap) -> Result<()> {
        let body = to_vec(map)?;

        self.wind(204, Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditVoiceState {
                guild_id,
                user_id,
            },
        })
        .await
    }

    /// Changes the current user's voice state in a stage channel.
    ///
    /// The Value is a map with values of:
    ///
    /// - **channel_id**: ID of the channel the user is currently in
    ///   (**required**)
    /// - **supress**: Bool which toggles user's suppressed state. Setting this
    ///   to `false` will invite the user to speak.
    /// - **request_to_speak_timestamp**: ISO8601 timestamp to set the user's
    ///   request to speak. This can be any present or future time.
    ///
    /// # Example
    ///
    /// Unsuppress the current bot user
    ///
    /// ```rust,no_run
    /// use serenity::{
    ///     http::Http,
    ///     json::{json, prelude::*},
    /// };
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let guild_id = 187450744427773963;
    /// let value = json!({
    ///     "channel_id": "826929611849334784",
    ///     "suppress": false,
    ///     "request_to_speak_timestamp": "2021-03-31T18:45:31.297561+00:00"
    /// });
    ///
    /// let map = value.as_object().unwrap();
    ///
    /// // Edit state for current user
    /// http.edit_voice_state_me(guild_id, &map).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn edit_voice_state_me(&self, guild_id: u64, map: &JsonMap) -> Result<()> {
        let body = to_vec(map)?;

        self.wind(204, Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditVoiceStateMe {
                guild_id,
            },
        })
        .await
    }

    /// Edits a the webhook with the given data.
    ///
    /// The Value is a map with optional values of:
    ///
    /// - **avatar**: base64-encoded 128x128 image for the webhook's default avatar
    ///   (_optional_);
    /// - **name**: the name of the webhook, limited to between 2 and 100 characters
    ///   long.
    ///
    /// Note that, unlike with [`Self::create_webhook`], _all_ values are optional.
    ///
    /// This method requires authentication, whereas [`Self::edit_webhook_with_token`]
    /// does not.
    ///
    /// # Examples
    ///
    /// Edit the image of a webhook given its Id and unique token:
    ///
    /// ```rust,no_run
    /// use serenity::json::json;
    /// use serenity::http::Http;
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let id = 245037420704169985;
    /// let image = serenity::utils::read_image("./webhook_img.png")?;
    /// let map = json!({
    ///     "avatar": image,
    /// });
    ///
    /// let edited = http.edit_webhook(id, &map, None).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn edit_webhook(
        &self,
        webhook_id: u64,
        map: &Value,
        audit_log_reason: Option<&str>,
    ) -> Result<Webhook> {
        self.fire(Request {
            body: Some(map.to_string().as_bytes()),
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::EditWebhook {
                webhook_id,
            },
        })
        .await
    }

    /// Edits the webhook with the given data.
    ///
    /// Refer to the documentation for [`Self::edit_webhook`] for more information.
    ///
    /// This method does _not_ require authentication.
    ///
    /// # Examples
    ///
    /// Edit the name of a webhook given its Id and unique token:
    ///
    /// ```rust,no_run
    /// use serenity::json::prelude::*;
    /// use serenity::http::Http;
    ///
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let id = 245037420704169985;
    /// let token = "ig5AO-wdVWpCBtUUMxmgsWryqgsW3DChbKYOINftJ4DCrUbnkedoYZD0VOH1QLr-S3sV";
    /// let value = json!({"name": "new name"});
    /// let map = value.as_object().unwrap();
    ///
    /// let edited = http.edit_webhook_with_token(id, token, map).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn edit_webhook_with_token(
        &self,
        webhook_id: u64,
        token: &str,
        map: &JsonMap,
    ) -> Result<Webhook> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditWebhookWithToken {
                token,
                webhook_id,
            },
        })
        .await
    }

    /// Executes a webhook, posting a [`Message`] in the webhook's associated
    /// [`Channel`].
    ///
    /// This method does _not_ require authentication.
    ///
    /// Pass `true` to `wait` to wait for server confirmation of the message sending
    /// before receiving a response. From the [Discord docs]:
    ///
    /// > waits for server confirmation of message send before response, and returns
    /// > the created message body (defaults to false; when false a message that is
    /// > not saved does not return an error)
    ///
    /// The map can _optionally_ contain the following data:
    ///
    /// - `avatar_url`: Override the default avatar of the webhook with a URL.
    /// - `tts`: Whether this is a text-to-speech message (defaults to `false`).
    /// - `username`: Override the default username of the webhook.
    ///
    /// Additionally, _at least one_ of the following must be given:
    ///
    /// - `content`: The content of the message.
    /// - `embeds`: An array of rich embeds.
    ///
    /// **Note**: For embed objects, all fields are registered by Discord except for
    /// `height`, `provider`, `proxy_url`, `type` (it will always be `rich`),
    /// `video`, and `width`. The rest will be determined by Discord.
    ///
    /// # Examples
    ///
    /// Sending a webhook with message content of `test`:
    ///
    /// ```rust,no_run
    /// use serenity::json::prelude::*;
    /// use serenity::http::Http;
    ///
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let id = 245037420704169985;
    /// let token = "ig5AO-wdVWpCBtUUMxmgsWryqgsW3DChbKYOINftJ4DCrUbnkedoYZD0VOH1QLr-S3sV";
    /// let value = json!({"content": "test"});
    /// let map = value.as_object().unwrap();
    ///
    /// let message = http.execute_webhook(id, token, true, map).await?;
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// [Discord docs]: https://discord.com/developers/docs/resources/webhook#querystring-params
    pub async fn execute_webhook(
        &self,
        webhook_id: u64,
        token: &str,
        wait: bool,
        map: &JsonMap,
    ) -> Result<Option<Message>> {
        let body = to_vec(map)?;

        let mut headers = Headers::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .request(Request {
                body: Some(&body),
                multipart: None,
                headers: Some(headers),
                route: RouteInfo::ExecuteWebhook {
                    token,
                    wait,
                    webhook_id,
                },
            })
            .await?;

        if response.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }

        response.json::<Message>().await.map(Some).map_err(From::from)
    }

    /// Send file(s) over a webhook.
    ///
    /// # Errors
    ///
    /// Returns an
    /// [`HttpError::UnsuccessfulRequest(ErrorResponse)`][`HttpError::UnsuccessfulRequest`]
    /// if the files are too large to send.
    pub async fn execute_webhook_with_files<'a, T, It: IntoIterator<Item = T>>(
        &self,
        webhook_id: u64,
        token: &str,
        wait: bool,
        files: It,
        map: JsonMap,
    ) -> Result<Option<Message>>
    where
        T: Into<AttachmentType<'a>>,
    {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: files.into_iter().map(Into::into).collect(),
                payload_json: Some(to_value(map)?),
                fields: vec![],
            }),
            headers: None,
            route: RouteInfo::ExecuteWebhook {
                token,
                wait,
                webhook_id,
            },
        })
        .await
    }

    /// Edits a webhook's message by Id.
    pub async fn edit_webhook_message(
        &self,
        webhook_id: u64,
        token: &str,
        message_id: u64,
        map: &JsonMap,
    ) -> Result<Message> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::EditWebhookMessage {
                token,
                webhook_id,
                message_id,
            },
        })
        .await
    }

    /// Deletes a webhook's messsage by Id.
    pub async fn delete_webhook_message(
        &self,
        webhook_id: u64,
        token: &str,
        message_id: u64,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::DeleteWebhookMessage {
                token,
                webhook_id,
                message_id,
            },
        })
        .await
    }

    /// Gets the active maintenances from Discord's Status API.
    ///
    /// Does not require authentication.
    pub async fn get_active_maintenances(&self) -> Result<Vec<Maintenance>> {
        let response = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetActiveMaintenance,
            })
            .await?;

        let mut map: BTreeMap<String, Value> = response.json::<BTreeMap<String, Value>>().await?;

        match map.remove("scheduled_maintenances") {
            Some(v) => from_value::<Vec<Maintenance>>(v).map_err(From::from),
            None => Ok(vec![]),
        }
    }

    /// Gets all the users that are banned in specific guild.
    pub async fn get_bans(&self, guild_id: u64) -> Result<Vec<Ban>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetBans {
                guild_id,
            },
        })
        .await
    }

    /// Gets all audit logs in a specific guild.
    pub async fn get_audit_logs(
        &self,
        guild_id: u64,
        action_type: Option<u8>,
        user_id: Option<u64>,
        before: Option<u64>,
        limit: Option<u8>,
    ) -> Result<AuditLogs> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetAuditLogs {
                action_type,
                before,
                guild_id,
                limit,
                user_id,
            },
        })
        .await
    }

    /// Gets current bot gateway.
    pub async fn get_bot_gateway(&self) -> Result<BotGateway> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetBotGateway,
        })
        .await
    }

    /// Gets all invites for a channel.
    pub async fn get_channel_invites(&self, channel_id: u64) -> Result<Vec<RichInvite>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelInvites {
                channel_id,
            },
        })
        .await
    }

    /// Gets all thread members for a thread.
    pub async fn get_channel_thread_members(&self, channel_id: u64) -> Result<Vec<ThreadMember>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelThreadMembers {
                channel_id,
            },
        })
        .await
    }

    /// Gets all active threads from a channel.
    pub async fn get_channel_active_threads(&self, channel_id: u64) -> Result<ThreadsData> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelActiveThreads {
                channel_id,
            },
        })
        .await
    }

    /// Gets all archived public threads from a channel.
    pub async fn get_channel_archived_public_threads(
        &self,
        channel_id: u64,
        before: Option<u64>,
        limit: Option<u64>,
    ) -> Result<ThreadsData> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelArchivedPublicThreads {
                channel_id,
                before,
                limit,
            },
        })
        .await
    }

    /// Gets all archived private threads from a channel.
    pub async fn get_channel_archived_private_threads(
        &self,
        channel_id: u64,
        before: Option<u64>,
        limit: Option<u64>,
    ) -> Result<ThreadsData> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelArchivedPrivateThreads {
                channel_id,
                before,
                limit,
            },
        })
        .await
    }

    /// Gets all archived private threads joined from a channel.
    pub async fn get_channel_joined_archived_private_threads(
        &self,
        channel_id: u64,
        before: Option<u64>,
        limit: Option<u64>,
    ) -> Result<ThreadsData> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelJoinedPrivateArchivedThreads {
                channel_id,
                before,
                limit,
            },
        })
        .await
    }

    /// Joins a thread channel.
    pub async fn join_thread_channel(&self, channel_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::JoinThread {
                channel_id,
            },
        })
        .await
    }

    /// Leaves a thread channel.
    pub async fn leave_thread_channel(&self, channel_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::LeaveThread {
                channel_id,
            },
        })
        .await
    }

    /// Adds a member to a thread channel.
    pub async fn add_thread_channel_member(&self, channel_id: u64, user_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::AddThreadMember {
                channel_id,
                user_id,
            },
        })
        .await
    }

    /// Removes a member from a thread channel.
    pub async fn remove_thread_channel_member(&self, channel_id: u64, user_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::RemoveThreadMember {
                channel_id,
                user_id,
            },
        })
        .await
    }

    /// Retrieves the webhooks for the given [channel][`GuildChannel`]'s Id.
    ///
    /// This method requires authentication.
    ///
    /// # Examples
    ///
    /// Retrieve all of the webhooks owned by a channel:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// # let http = Http::default();
    /// let channel_id = 81384788765712384;
    ///
    /// let webhooks = http.get_channel_webhooks(channel_id).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn get_channel_webhooks(&self, channel_id: u64) -> Result<Vec<Webhook>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannelWebhooks {
                channel_id,
            },
        })
        .await
    }

    /// Gets channel information.
    pub async fn get_channel(&self, channel_id: u64) -> Result<Channel> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannel {
                channel_id,
            },
        })
        .await
    }

    /// Gets all channels in a guild.
    pub async fn get_channels(&self, guild_id: u64) -> Result<Vec<GuildChannel>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetChannels {
                guild_id,
            },
        })
        .await
    }

    /// Gets a stage instance.
    pub async fn get_stage_instance(&self, channel_id: u64) -> Result<StageInstance> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetStageInstance {
                channel_id,
            },
        })
        .await
    }

    /// Gets information about the current application.
    ///
    /// **Note**: Only applications may use this endpoint.
    pub async fn get_current_application_info(&self) -> Result<CurrentApplicationInfo> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetCurrentApplicationInfo,
        })
        .await
    }

    /// Gets information about the user we're connected with.
    pub async fn get_current_user(&self) -> Result<CurrentUser> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetCurrentUser,
        })
        .await
    }

    /// Gets all emojis of a guild.
    pub async fn get_emojis(&self, guild_id: u64) -> Result<Vec<Emoji>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetEmojis {
                guild_id,
            },
        })
        .await
    }

    /// Gets information about an emoji in a guild.
    pub async fn get_emoji(&self, guild_id: u64, emoji_id: u64) -> Result<Emoji> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetEmoji {
                guild_id,
                emoji_id,
            },
        })
        .await
    }

    /// Gets current gateway.
    pub async fn get_gateway(&self) -> Result<Gateway> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGateway,
        })
        .await
    }

    /// Fetches all of the global commands for your application.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_global_application_commands(&self) -> Result<Vec<ApplicationCommand>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGlobalApplicationCommands {
                application_id: self.application_id,
            },
        })
        .await
    }

    /// Fetches a global commands for your application by its Id.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_global_application_command(
        &self,
        command_id: u64,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGlobalApplicationCommand {
                application_id: self.application_id,
                command_id,
            },
        })
        .await
    }

    /// Gets guild information.
    pub async fn get_guild(&self, guild_id: u64) -> Result<PartialGuild> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuild {
                guild_id,
            },
        })
        .await
    }

    /// Gets guild information with counts.
    pub async fn get_guild_with_counts(&self, guild_id: u64) -> Result<PartialGuild> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildWithCounts {
                guild_id,
            },
        })
        .await
    }

    /// Fetches all of the guild commands for your application for a specific guild.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_guild_application_commands(
        &self,
        guild_id: u64,
    ) -> Result<Vec<ApplicationCommand>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildApplicationCommands {
                application_id: self.application_id,
                guild_id,
            },
        })
        .await
    }

    /// Fetches a guild command by its Id.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_guild_application_command(
        &self,
        guild_id: u64,
        command_id: u64,
    ) -> Result<ApplicationCommand> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildApplicationCommand {
                application_id: self.application_id,
                guild_id,
                command_id,
            },
        })
        .await
    }

    /// Fetches all of the guild commands permissions for your application for a specific guild.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_guild_application_commands_permissions(
        &self,
        guild_id: u64,
    ) -> Result<Vec<ApplicationCommandPermission>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildApplicationCommandsPermissions {
                application_id: self.application_id,
                guild_id,
            },
        })
        .await
    }

    /// Gives the guild command permission for your application for a specific guild.
    #[cfg(feature = "unstable_discord_api")]
    #[cfg_attr(docsrs, doc(cfg(feature = "unstable_discord_api")))]
    pub async fn get_guild_application_command_permissions(
        &self,
        guild_id: u64,
        command_id: u64,
    ) -> Result<ApplicationCommandPermission> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildApplicationCommandPermissions {
                application_id: self.application_id,
                guild_id,
                command_id,
            },
        })
        .await
    }

    /// Gets a guild embed information.
    #[deprecated(note = "get_guild_embed was renamed to get_guild_widget")]
    #[allow(deprecated)]
    pub async fn get_guild_embed(&self, guild_id: u64) -> Result<GuildEmbed> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildWidget {
                guild_id,
            },
        })
        .await
    }

    /// Gets a guild widget information.
    pub async fn get_guild_widget(&self, guild_id: u64) -> Result<GuildWidget> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildWidget {
                guild_id,
            },
        })
        .await
    }

    /// Gets a guild preview.
    pub async fn get_guild_preview(&self, guild_id: u64) -> Result<GuildPreview> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildPreview {
                guild_id,
            },
        })
        .await
    }

    /// Gets a guild welcome screen information.
    pub async fn get_guild_welcome_screen(&self, guild_id: u64) -> Result<GuildWelcomeScreen> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildWelcomeScreen {
                guild_id,
            },
        })
        .await
    }

    /// Gets integrations that a guild has.
    pub async fn get_guild_integrations(&self, guild_id: u64) -> Result<Vec<Integration>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildIntegrations {
                guild_id,
            },
        })
        .await
    }

    /// Gets all invites to a guild.
    pub async fn get_guild_invites(&self, guild_id: u64) -> Result<Vec<RichInvite>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildInvites {
                guild_id,
            },
        })
        .await
    }

    /// Gets a guild's vanity URL if it has one.
    pub async fn get_guild_vanity_url(&self, guild_id: u64) -> Result<String> {
        #[derive(Deserialize)]
        struct GuildVanityUrl {
            code: String,
        }

        self.request(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildVanityUrl {
                guild_id,
            },
        })
        .await?
        .json::<GuildVanityUrl>()
        .await
        .map(|x| x.code)
        .map_err(From::from)
    }

    /// Gets the members of a guild. Optionally pass a `limit` and the Id of the
    /// user to offset the result by.
    pub async fn get_guild_members(
        &self,
        guild_id: u64,
        limit: Option<u64>,
        after: Option<u64>,
    ) -> Result<Vec<Member>> {
        if let Some(l) = limit {
            if !(1..=constants::MEMBER_FETCH_LIMIT).contains(&l) {
                return Err(Error::NotInRange("limit", l, 1, constants::MEMBER_FETCH_LIMIT));
            }
        }

        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetGuildMembers {
                    after,
                    guild_id,
                    limit,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(values) = value.as_array_mut() {
            let num = from_number(guild_id);

            for value in values {
                if let Some(element) = value.as_object_mut() {
                    element.insert("guild_id".to_string(), num.clone());
                }
            }
        }

        from_value::<Vec<Member>>(value).map_err(From::from)
    }

    /// Gets the amount of users that can be pruned.
    pub async fn get_guild_prune_count(&self, guild_id: u64, map: &Value) -> Result<GuildPrune> {
        // Note for 0.6.x: turn this into a function parameter.
        #[derive(Deserialize)]
        struct GetGuildPruneCountRequest {
            days: u64,
        }

        let req = from_value::<GetGuildPruneCountRequest>(map.clone())?;

        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildPruneCount {
                days: req.days,
                guild_id,
            },
        })
        .await
    }

    /// Gets regions that a guild can use. If a guild has the `VIP_REGIONS` feature
    /// enabled, then additional VIP-only regions are returned.
    pub async fn get_guild_regions(&self, guild_id: u64) -> Result<Vec<VoiceRegion>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildRegions {
                guild_id,
            },
        })
        .await
    }

    /// Retrieves a list of roles in a [`Guild`].
    pub async fn get_guild_roles(&self, guild_id: u64) -> Result<Vec<Role>> {
        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetGuildRoles {
                    guild_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(array) = value.as_array_mut() {
            for sticker in array {
                if let Some(map) = sticker.as_object_mut() {
                    map.insert("guild_id".to_string(), from_number(guild_id));
                }
            }
        }

        from_value(value).map_err(From::from)
    }

    /// Retrieves a list of stickers in a [`Guild`].
    pub async fn get_guild_stickers(&self, guild_id: u64) -> Result<Vec<Sticker>> {
        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetGuildStickers {
                    guild_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(array) = value.as_array_mut() {
            for role in array {
                if let Some(map) = role.as_object_mut() {
                    map.insert("guild_id".to_string(), from_number(guild_id));
                }
            }
        }

        from_value(value).map_err(From::from)
    }

    /// Retrieves a single sticker in a [`Guild`].
    pub async fn get_guild_sticker(&self, guild_id: u64, sticker_id: u64) -> Result<Sticker> {
        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetGuildSticker {
                    guild_id,
                    sticker_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value(value).map_err(From::from)
    }

    /// Retrieves the webhooks for the given [guild][`Guild`]'s Id.
    ///
    /// This method requires authentication.
    ///
    /// # Examples
    ///
    /// Retrieve all of the webhooks owned by a guild:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let guild_id = 81384788765712384;
    ///
    /// let webhooks = http.get_guild_webhooks(guild_id).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn get_guild_webhooks(&self, guild_id: u64) -> Result<Vec<Webhook>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuildWebhooks {
                guild_id,
            },
        })
        .await
    }

    /// Gets a paginated list of the current user's guilds.
    ///
    /// The `limit` has a maximum value of 100.
    ///
    /// [Discord's documentation][docs]
    ///
    /// # Examples
    ///
    /// Get the first 10 guilds after a certain guild's Id:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// use serenity::{http::GuildPagination, model::id::GuildId};
    ///
    /// let guild_id = GuildId(81384788765712384);
    ///
    /// let guilds = http.get_guilds(Some(&GuildPagination::After(guild_id)), Some(10)).await?;
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// [docs]: https://discord.com/developers/docs/resources/user#get-current-user-guilds
    pub async fn get_guilds(
        &self,
        target: Option<&GuildPagination>,
        limit: Option<u64>,
    ) -> Result<Vec<GuildInfo>> {
        let (after, before) = match target {
            None => (None, None),
            Some(gp) => match gp {
                GuildPagination::After(id) => (Some(id.0), None),
                GuildPagination::Before(id) => (None, Some(id.0)),
            },
        };

        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetGuilds {
                after,
                before,
                limit,
            },
        })
        .await
    }

    /// Gets information about a specific invite.
    pub async fn get_invite(&self, mut code: &str, stats: bool) -> Result<Invite> {
        #[cfg(feature = "utils")]
        {
            code = crate::utils::parse_invite(code);
        }

        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetInvite {
                code,
                stats,
            },
        })
        .await
    }

    /// Gets member of a guild.
    pub async fn get_member(&self, guild_id: u64, user_id: u64) -> Result<Member> {
        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetMember {
                    guild_id,
                    user_id,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(map) = value.as_object_mut() {
            map.insert("guild_id".to_string(), from_number(guild_id));
        }

        from_value::<Member>(value).map_err(From::from)
    }

    /// Gets a message by an Id, bots only.
    pub async fn get_message(&self, channel_id: u64, message_id: u64) -> Result<Message> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Gets X messages from a channel.
    pub async fn get_messages(&self, channel_id: u64, query: &str) -> Result<Vec<Message>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetMessages {
                query: query.to_owned(),
                channel_id,
            },
        })
        .await
    }

    /// Retrieves a list of all nitro sticker packs.
    pub async fn get_nitro_stickers(&self) -> Result<Vec<StickerPack>> {
        #[derive(Deserialize)]
        struct StickerPacks {
            sticker_packs: Vec<StickerPack>,
        }

        self.request(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetStickerPacks,
        })
        .await?
        .json::<StickerPacks>()
        .await
        .map(|s| s.sticker_packs)
        .map_err(From::from)
    }

    /// Gets all pins of a channel.
    pub async fn get_pins(&self, channel_id: u64) -> Result<Vec<Message>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetPins {
                channel_id,
            },
        })
        .await
    }

    /// Gets user Ids based on their reaction to a message. This endpoint is dumb.
    pub async fn get_reaction_users(
        &self,
        channel_id: u64,
        message_id: u64,
        reaction_type: &ReactionType,
        limit: u8,
        after: Option<u64>,
    ) -> Result<Vec<User>> {
        let reaction = reaction_type.as_data();

        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetReactionUsers {
                after,
                channel_id,
                limit,
                message_id,
                reaction,
            },
        })
        .await
    }

    /// Gets a sticker.
    pub async fn get_sticker(&self, sticker_id: u64) -> Result<Sticker> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetSticker {
                sticker_id,
            },
        })
        .await
    }

    /// Gets the current unresolved incidents from Discord's Status API.
    ///
    /// Does not require authentication.
    pub async fn get_unresolved_incidents(&self) -> Result<Vec<Incident>> {
        let response = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetUnresolvedIncidents,
            })
            .await?;

        let mut map = response.json::<BTreeMap<String, Value>>().await?;

        match map.remove("incidents") {
            Some(v) => from_value::<Vec<Incident>>(v).map_err(From::from),
            None => Ok(vec![]),
        }
    }

    /// Gets the upcoming (planned) maintenances from Discord's Status API.
    ///
    /// Does not require authentication.
    pub async fn get_upcoming_maintenances(&self) -> Result<Vec<Maintenance>> {
        let response = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::GetUpcomingMaintenances,
            })
            .await?;

        let mut map = response.json::<BTreeMap<String, Value>>().await?;

        match map.remove("scheduled_maintenances") {
            Some(v) => from_value::<Vec<Maintenance>>(v).map_err(From::from),
            None => Ok(vec![]),
        }
    }

    /// Gets a user by Id.
    pub async fn get_user(&self, user_id: u64) -> Result<User> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetUser {
                user_id,
            },
        })
        .await
    }

    /// Gets the current user's third party connections.
    ///
    /// This method only works for user tokens with the
    /// [`Connections`] OAuth2 scope.
    ///
    /// [`Connections`]: crate::model::oauth2::OAuth2Scope::Connections
    pub async fn get_user_connections(&self) -> Result<Vec<Connection>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetUserConnections,
        })
        .await
    }

    /// Gets our DM channels.
    pub async fn get_user_dm_channels(&self) -> Result<Vec<PrivateChannel>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetUserDmChannels,
        })
        .await
    }

    /// Gets all voice regions.
    pub async fn get_voice_regions(&self) -> Result<Vec<VoiceRegion>> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetVoiceRegions,
        })
        .await
    }

    /// Retrieves a webhook given its Id.
    ///
    /// This method requires authentication, whereas [`Self::get_webhook_with_token`] does
    /// not.
    ///
    /// # Examples
    ///
    /// Retrieve a webhook by Id:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let id = 245037420704169985;
    /// let webhook = http.get_webhook(id).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn get_webhook(&self, webhook_id: u64) -> Result<Webhook> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetWebhook {
                webhook_id,
            },
        })
        .await
    }

    /// Retrieves a webhook given its Id and unique token.
    ///
    /// This method does _not_ require authentication.
    ///
    /// # Examples
    ///
    /// Retrieve a webhook by Id and its unique token:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// let id = 245037420704169985;
    /// let token = "ig5AO-wdVWpCBtUUMxmgsWryqgsW3DChbKYOINftJ4DCrUbnkedoYZD0VOH1QLr-S3sV";
    ///
    /// let webhook = http.get_webhook_with_token(id, token).await?;
    /// #     Ok(())
    /// # }
    /// ```
    pub async fn get_webhook_with_token(&self, webhook_id: u64, token: &str) -> Result<Webhook> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::GetWebhookWithToken {
                token,
                webhook_id,
            },
        })
        .await
    }

    /// Kicks a member from a guild.
    pub async fn kick_member(&self, guild_id: u64, user_id: u64) -> Result<()> {
        self.kick_member_with_reason(guild_id, user_id, "", None).await
    }

    /// Kicks a member from a guild with a provided reason.
    pub async fn kick_member_with_reason(
        &self,
        guild_id: u64,
        user_id: u64,
        reason: &str,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::KickMember {
                guild_id,
                user_id,
                reason: &utf8_percent_encode(reason, NON_ALPHANUMERIC).to_string(),
            },
        })
        .await
    }

    /// Leaves a guild.
    pub async fn leave_guild(&self, guild_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::LeaveGuild {
                guild_id,
            },
        })
        .await
    }

    /// Sends file(s) to a channel.
    ///
    /// # Errors
    ///
    /// Returns an
    /// [`HttpError::UnsuccessfulRequest(ErrorResponse)`][`HttpError::UnsuccessfulRequest`]
    /// if the files are too large to send.
    pub async fn send_files<'a, T, It: IntoIterator<Item = T>>(
        &self,
        channel_id: u64,
        files: It,
        map: JsonMap,
    ) -> Result<Message>
    where
        T: Into<AttachmentType<'a>>,
    {
        self.fire(Request {
            body: None,
            multipart: Some(Multipart {
                files: files.into_iter().map(Into::into).collect(),
                payload_json: Some(to_value(map)?),
                fields: vec![],
            }),
            headers: None,
            route: RouteInfo::CreateMessage {
                channel_id,
            },
        })
        .await
    }

    /// Sends a message to a channel.
    pub async fn send_message(&self, channel_id: u64, map: &Value) -> Result<Message> {
        let body = to_vec(map)?;

        self.fire(Request {
            body: Some(&body),
            multipart: None,
            headers: None,
            route: RouteInfo::CreateMessage {
                channel_id,
            },
        })
        .await
    }

    /// Pins a message in a channel.
    pub async fn pin_message(
        &self,
        channel_id: u64,
        message_id: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::PinMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Unbans a user from a guild.
    pub async fn remove_ban(
        &self,
        guild_id: u64,
        user_id: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::RemoveBan {
                guild_id,
                user_id,
            },
        })
        .await
    }

    /// Deletes a single [`Role`] from a [`Member`] in a [`Guild`].
    ///
    /// **Note**: Requires the [Manage Roles] permission and respect of role
    /// hierarchy.
    ///
    /// [Manage Roles]: Permissions::MANAGE_ROLES
    pub async fn remove_member_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::RemoveMemberRole {
                guild_id,
                user_id,
                role_id,
            },
        })
        .await
    }

    /// Returns a list of [`Member`]s in a [`Guild`] whose username or nickname
    /// starts with a provided string.
    pub async fn search_guild_members(
        &self,
        guild_id: u64,
        query: &str,
        limit: Option<u64>,
    ) -> Result<Vec<Member>> {
        let mut value = self
            .request(Request {
                body: None,
                multipart: None,
                headers: None,
                route: RouteInfo::SearchGuildMembers {
                    guild_id,
                    query,
                    limit,
                },
            })
            .await?
            .json::<Value>()
            .await?;

        if let Some(members) = value.as_array_mut() {
            for member in members {
                if let Some(map) = member.as_object_mut() {
                    map.insert("guild_id".to_string(), from_number(guild_id));
                }
            }
        }

        from_value(value).map_err(From::from)
    }

    /// Starts removing some members from a guild based on the last time they've been online.
    pub async fn start_guild_prune(
        &self,
        guild_id: u64,
        days: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<GuildPrune> {
        self.fire(Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::StartGuildPrune {
                days,
                guild_id,
            },
        })
        .await
    }

    /// Starts syncing an integration with a guild.
    pub async fn start_integration_sync(&self, guild_id: u64, integration_id: u64) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: None,
            route: RouteInfo::StartIntegrationSync {
                guild_id,
                integration_id,
            },
        })
        .await
    }

    /// Starts typing in the specified [`Channel`] for an indefinite period of time.
    ///
    /// Returns [`Typing`] that is used to trigger the typing. [`Typing::stop`] must be called
    /// on the returned struct to stop typing. Note that on some clients, typing may persist
    /// for a few seconds after [`Typing::stop`] is called.
    /// Typing is also stopped when the struct is dropped.
    ///
    /// If a message is sent while typing is triggered, the user will stop typing for a brief period
    /// of time and then resume again until either [`Typing::stop`] is called or the struct is dropped.
    ///
    /// This should rarely be used for bots, although it is a good indicator that a
    /// long-running command is still being processed.
    ///
    /// ## Examples
    ///
    /// ```rust,no_run
    /// # use serenity::{http::{Http, Typing}, Result};
    /// # use std::sync::Arc;
    /// #
    /// # fn long_process() {}
    /// # fn main() -> Result<()> {
    /// # let http = Arc::new(Http::default());
    /// // Initiate typing (assuming http is `Arc<Http>`)
    /// let typing = http.start_typing(7)?;
    ///
    /// // Run some long-running process
    /// long_process();
    ///
    /// // Stop typing
    /// typing.stop();
    /// #
    /// # Ok(())
    /// # }
    /// ```
    pub fn start_typing(self: &Arc<Self>, channel_id: u64) -> Result<Typing> {
        Typing::start(self.clone(), channel_id)
    }

    /// Unpins a message from a channel.
    pub async fn unpin_message(
        &self,
        channel_id: u64,
        message_id: u64,
        audit_log_reason: Option<&str>,
    ) -> Result<()> {
        self.wind(204, Request {
            body: None,
            multipart: None,
            headers: audit_log_reason.map(reason_into_header),
            route: RouteInfo::UnpinMessage {
                channel_id,
                message_id,
            },
        })
        .await
    }

    /// Fires off a request, deserializing the response reader via the given type
    /// bound.
    ///
    /// If you don't need to deserialize the response and want the response instance
    /// itself, use [`Self::request`].
    ///
    /// # Examples
    ///
    /// Create a new message via the [`RouteInfo::CreateMessage`] endpoint and
    /// deserialize the response into a [`Message`]:
    ///
    /// ```rust,no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// # use serenity::http::Http;
    /// #
    /// # let http = Http::default();
    /// use serenity::{
    ///     http::{
    ///         routing::RouteInfo,
    ///         request::RequestBuilder,
    ///     },
    ///     model::channel::Message,
    /// };
    ///
    /// let bytes = vec![
    ///     // payload bytes here
    /// ];
    /// let channel_id = 381880193700069377;
    /// let route_info = RouteInfo::CreateMessage { channel_id };
    ///
    /// let mut request = RequestBuilder::new(route_info);
    /// request.body(Some(&bytes));
    ///
    /// let message = http.fire::<Message>(request.build()).await?;
    ///
    /// println!("Message content: {}", message.content);
    /// #
    /// #     Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// If there is an error, it will be either [`Error::Http`] or [`Error::Json`].
    ///
    /// [`Error::Http`]: crate::error::Error::Http
    /// [`Error::Json`]: crate::error::Error::Json
    pub async fn fire<T: DeserializeOwned>(&self, req: Request<'_>) -> Result<T> {
        let response = self.request(req).await?;

        response.json::<T>().await.map_err(From::from)
    }

    /// Performs a request, ratelimiting it if necessary.
    ///
    /// Returns the raw reqwest Response. Use [`Self::fire`] to deserialize the response
    /// into some type.
    ///
    /// # Examples
    ///
    /// Send a body of bytes over the [`RouteInfo::CreateMessage`] endpoint:
    ///
    /// ```rust,no_run
    /// # use serenity::http::Http;
    /// #
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// #     let http = Http::default();
    /// use serenity::http::{
    ///     request::RequestBuilder,
    ///     routing::RouteInfo,
    /// };
    ///
    /// let bytes = vec![
    ///     // payload bytes here
    /// ];
    /// let channel_id = 381880193700069377;
    /// let route_info = RouteInfo::CreateMessage { channel_id };
    ///
    /// let mut request = RequestBuilder::new(route_info);
    /// request.body(Some(&bytes));
    ///
    /// let response = http.request(request.build()).await?;
    ///
    /// println!("Response successful?: {}", response.status().is_success());
    /// #
    /// #     Ok(())
    /// # }
    /// ```
    #[instrument]
    pub async fn request(&self, mut req: Request<'_>) -> Result<ReqwestResponse> {
        let response = if self.ratelimiter_disabled {
            let request =
                req.build(&self.client, &self.token, self.proxy.as_ref()).await?.build()?;
            self.client.execute(request).await?
        } else {
            let ratelimiting_req = RatelimitedRequest::from(req);
            self.ratelimiter.perform(ratelimiting_req).await?
        };

        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Error::Http(Box::new(HttpError::from_response(response).await)))
        }
    }

    /// Performs a request and then verifies that the response status code is equal
    /// to the expected value.
    ///
    /// This is a function that performs a light amount of work and returns an
    /// empty tuple, so it's called "self.wind" to denote that it's lightweight.
    pub(super) async fn wind(&self, expected: u16, req: Request<'_>) -> Result<()> {
        let response = self.request(req).await?;

        if response.status().as_u16() == expected {
            return Ok(());
        }

        debug!("Expected {}, got {}", expected, response.status());
        trace!("Unsuccessful response: {:?}", response);

        Err(Error::Http(Box::new(HttpError::from_response(response).await)))
    }
}

#[cfg(not(feature = "native_tls_backend_marker"))]
fn configure_client_backend(builder: ClientBuilder) -> ClientBuilder {
    builder.use_rustls_tls()
}

#[cfg(feature = "native_tls_backend_marker")]
fn configure_client_backend(builder: ClientBuilder) -> ClientBuilder {
    builder.use_native_tls()
}

impl AsRef<Http> for Http {
    fn as_ref(&self) -> &Http {
        self
    }
}

impl Default for Http {
    fn default() -> Self {
        let built = Client::builder().build().expect("Cannot build Reqwest::Client.");
        let client = Arc::new(built);
        let client2 = Arc::clone(&client);

        Self {
            client,
            ratelimiter: Ratelimiter::new(client2, ""),
            ratelimiter_disabled: false,
            proxy: None,
            token: "".to_string(),
            #[cfg(feature = "unstable_discord_api")]
            application_id: 0,
        }
    }
}
