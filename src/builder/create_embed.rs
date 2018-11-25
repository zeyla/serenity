//! Developer note:
//!
//! This is a set of embed builders for rich embeds.
//!
//! These are used in the [`ChannelId::send_message`] and
//! [`ExecuteWebhook::embeds`] methods, both as part of builders.
//!
//! The only builder that should be exposed is [`CreateEmbed`]. The rest of
//! these have no real reason for being exposed, but are for completeness' sake.
//!
//! Documentation for embeds can be found [here].
//!
//! [`ChannelId::send_message`]: ../model/id/struct.ChannelId.html#method.send_message
//! [`CreateEmbed`]: struct.CreateEmbed.html
//! [`ExecuteWebhook::embeds`]: struct.ExecuteWebhook.html#method.embeds
//! [here]: https://discordapp.com/developers/docs/resources/channel#embed-object

use chrono::{DateTime, TimeZone};
use internal::prelude::*;
use model::channel::Embed;
use serde_json::Value;
use std::{
    default::Default,
    fmt::Display
};
use utils::{self, VecMap};

#[cfg(feature = "utils")]
use utils::Colour;

/// A builder to create a fake [`Embed`] object, for use with the
/// [`ChannelId::send_message`] and [`ExecuteWebhook::embeds`] methods.
///
/// # Examples
///
/// Refer to the documentation for [`ChannelId::send_message`] for a very in-depth
/// example on how to use this.
///
/// [`ChannelId::send_message`]: ../model/id/struct.ChannelId.html#method.send_message
/// [`Embed`]: ../model/channel/struct.Embed.html
/// [`ExecuteWebhook::embeds`]: struct.ExecuteWebhook.html#method.embeds
#[derive(Clone, Debug)]
pub struct CreateEmbed(pub VecMap<&'static str, Value>);

impl CreateEmbed {
    /// Set the author of the embed.
    ///
    /// Refer to the documentation for [`CreateEmbedAuthor`] for more
    /// information.
    ///
    /// [`CreateEmbedAuthor`]: struct.CreateEmbedAuthor.html
    pub fn author<F>(&mut self, f: F) -> &mut Self
        where F: FnOnce(&mut CreateEmbedAuthor) -> &mut CreateEmbedAuthor {
        let mut author = CreateEmbedAuthor::default();
        f(&mut author);

        let map = utils::vecmap_to_json_map(author.0);

        self.0.insert("author", Value::Object(map));
        self
    }

    /// Set the colour of the left-hand side of the embed.
    ///
    /// This is an alias of [`colour`].
    ///
    /// [`colour`]: #method.colour
    #[cfg(feature = "utils")]
    #[inline]
    pub fn color<C: Into<Colour>>(&mut self, colour: C) -> &mut Self {
        self.colour(colour);
        self
    }

    /// Set the colour of the left-hand side of the embed.
    #[cfg(feature = "utils")]
    #[inline]
    pub fn colour<C: Into<Colour>>(&mut self, colour: C) -> &mut Self {
        self._colour(colour.into());
        self
    }

    #[cfg(feature = "utils")]
    fn _colour(&mut self, colour: Colour) {
        self.0.insert(
            "color",
            Value::Number(Number::from(u64::from(colour.0))),
        );
    }

    /// Set the colour of the left-hand side of the embed.
    ///
    /// This is an alias of [`colour`].
    ///
    /// [`colour`]: #method.colour
    #[cfg(not(feature = "utils"))]
    #[inline]
    pub fn color(&mut self, colour: u32) -> &mut Self {
        self.colour(colour);
        self
    }

    /// Set the colour of the left-hand side of the embed.
    #[cfg(not(feature = "utils"))]
    pub fn colour(&mut self, colour: u32) -> &mut Self {
        self.0.insert("color", Value::Number(Number::from(colour)));
        self
    }

    /// Set the description of the embed.
    ///
    /// **Note**: This can't be longer than 2048 characters.
    #[inline]
    pub fn description<D: Display>(&mut self, description: D) -> &mut Self {
        self._description(description.to_string());
        self
    }

    fn _description(&mut self, description: String) {
        self.0.insert(
            "description",
            Value::String(description),
        );
    }

    /// Set a field. Note that this will not overwrite other fields, and will
    /// add to them.
    ///
    /// **Note**: Maximum amount of characters you can put is 256 in a field
    /// name and 1024 in a field value.
    #[inline]
    pub fn field<T, U>(&mut self, name: T, value: U, inline: bool)  -> &mut Self
        where T: Display, U: Display {
        self._field(&name.to_string(), &value.to_string(), inline);
        self
    }

    fn _field(&mut self, name: &str, value: &str, inline: bool) {
        {
            let entry = self.0
                .entry("fields")
                .or_insert_with(|| Value::Array(vec![]));

            if let Value::Array(ref mut inner) = *entry {
                inner.push(json!({
                    "inline": inline,
                    "name": name,
                    "value": value,
                }));
            }
        }
    }

    /// Adds multiple fields at once.
    ///
    /// This is sugar to reduce the need of calling [`field`] manually multiple times.
    ///
    /// [`field`]: #method.field
    pub fn fields<T, U, It>(&mut self, fields: It) -> &mut Self
        where It: IntoIterator<Item=(T, U, bool)>,
              T: Display,
              U: Display {
        for field in fields {
            self.field(field.0.to_string(), field.1.to_string(), field.2);
        }
        self
    }

    /// Set the footer of the embed.
    ///
    /// Refer to the documentation for [`CreateEmbedFooter`] for more
    /// information.
    ///
    /// [`CreateEmbedFooter`]: struct.CreateEmbedFooter.html
    pub fn footer<F>(&mut self, f: F) -> &mut Self
        where F: FnOnce(&mut CreateEmbedFooter) -> &mut CreateEmbedFooter {
        let mut create_embed_footer = CreateEmbedFooter::default();
        f(&mut create_embed_footer);
        let footer = create_embed_footer.0;
        let map = utils::vecmap_to_json_map(footer);

        self.0.insert("footer", Value::Object(map));
        self
    }

    fn url_object(&mut self, name: &'static str, url: &str) -> &mut Self {
        let obj = json!({
            "url": url.to_string()
        });

        self.0.insert(name, obj);
        self
    }

    /// Set the image associated with the embed. This only supports HTTP(S).
    #[inline]
    pub fn image<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self._image(url.as_ref());
        self
    }

    fn _image(&mut self, url: &str) {
        self.url_object("image", url);
    }

    /// Set the thumbnail of the embed. This only supports HTTP(S).
    #[inline]
    pub fn thumbnail<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self._thumbnail(url.as_ref());
        self
    }

    fn _thumbnail(&mut self, url: &str) {
        self.url_object("thumbnail", url);
    }

    /// Set the timestamp.
    ///
    /// You may pass a direct string:
    ///
    /// - `2017-01-03T23:00:00`
    /// - `2004-06-08T16:04:23`
    /// - `2004-06-08T16:04:23`
    ///
    /// This timestamp must be in ISO-8601 format. It must also be in UTC format.
    ///
    /// You can also pass anything that implements `chrono::TimeZone`.
    ///
    /// # Examples
    ///
    /// Passing a string timestamp:
    ///
    /// ```rust,no_run
    /// use serenity::prelude::*;
    /// use serenity::model::channel::Message;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {
    ///     fn message(&self, _: Context, mut msg: Message) {
    ///         if msg.content == "~embed" {
    ///             let _ = msg.channel_id.send_message(|m| {
    ///                 m.embed(|e| {
    ///                     e.title("hello").timestamp("2004-06-08T16:04:23")
    ///                 });
    ///
    ///                 m
    ///             });
    ///         }
    ///     }
    /// }
    ///
    /// let mut client = Client::new("token", Handler).unwrap();
    ///
    /// client.start().unwrap();
    /// ```
    ///
    /// Creating a join-log:
    ///
    /// Note: this example isn't efficient and is for demonstrative purposes.
    ///
    /// ```rust,no_run
    /// use serenity::prelude::*;
    /// use serenity::model::guild::Member;
    /// use serenity::model::id::GuildId;
    ///
    /// struct Handler;
    ///
    /// impl EventHandler for Handler {
    ///     fn guild_member_addition(&self, _: Context, guild_id: GuildId, member: Member) {
    ///         use serenity::CACHE;
    ///         let cache = CACHE.read();
    ///
    ///         if let Some(guild) = cache.guild(guild_id) {
    ///             let guild = guild.read();
    ///
    ///             let channel_search = guild
    ///                 .channels
    ///                 .values()
    ///                 .find(|c| c.read().name == "join-log");
    ///
    ///             if let Some(channel) = channel_search {
    ///                 let user = member.user.read();
    ///
    ///                 let _ = channel.write().send_message(|m| {
    ///                     m.embed(|e| {
    ///                         e.author(|a| {
    ///                             a.icon_url(&user.face()).name(&user.name)
    ///                         });
    ///                         e.title("Member Join");
    ///
    ///                         if let Some(ref joined_at) = member.joined_at {
    ///                             e.timestamp(joined_at);
    ///                         }
    ///
    ///                         e
    ///                     })
    ///                 });
    ///             }
    ///         }
    ///     }
    /// }
    ///
    /// let mut client = Client::new("token", Handler).unwrap();
    ///
    /// client.start().unwrap();
    /// ```
    #[inline]
    pub fn timestamp<T: Into<Timestamp>>(&mut self, timestamp: T) -> &mut Self {
        self._timestamp(timestamp.into());
        self
    }

    fn _timestamp(&mut self, timestamp: Timestamp) {
        self.0.insert("timestamp", Value::String(timestamp.ts));
    }

    /// Set the title of the embed.
    #[inline]
    pub fn title<D: Display>(&mut self, title: D) -> &mut Self {
        self._title(title.to_string());
        self
    }

    fn _title(&mut self, title: String) {
        self.0.insert("title", Value::String(title));
    }

    /// Set the URL to direct to when clicking on the title.
    #[inline]
    pub fn url<S: AsRef<str>>(&mut self, url: S) -> &mut Self {
        self._url(url.as_ref());
        self
    }

    fn _url(&mut self, url: &str) {
        self.0.insert("url", Value::String(url.to_string()));
    }

    /// Same as calling [`image`] with "attachment://filename.(jpg, png)".
    ///
    /// Note however, you have to be sure you set an attachment (with [`ChannelId::send_files`])
    /// with the provided filename. Or else this won't work.
    ///
    /// [`ChannelId::send_files`]: ../model/id/struct.ChannelId.html#send_files
    ///
    /// [`image`]: #method.image
    #[inline]
    pub fn attachment<S: AsRef<str>>(&mut self, filename: S) -> &mut Self {
        self._attachment(filename.as_ref());
        self
    }

    fn _attachment(&mut self, filename: &str) {
        self.image(&format!("attachment://{}", filename));
    }
}

impl Default for CreateEmbed {
    /// Creates a builder with default values, setting the `type` to `rich`.
    fn default() -> CreateEmbed {
        let mut map = VecMap::new();
        map.insert("type", Value::String("rich".to_string()));

        CreateEmbed(map)
    }
}

impl From<Embed> for CreateEmbed {
    /// Converts the fields of an embed into the values for a new embed builder.
    ///
    /// Some values - such as Proxy URLs - are not preserved.
    fn from(embed: Embed) -> Self {
        let mut b = CreateEmbed::default();
        b.colour(embed.colour);

        if let Some(author) = embed.author {
            b.author(move |a| {
                a.name(&author.name);

                if let Some(icon_url) = author.icon_url {
                    a.icon_url(&icon_url);
                }

                if let Some(url) = author.url {
                    a.url(&url);
                }

                a
            });
        }

        if let Some(description) = embed.description {
            b.description(&description);
        }

        for field in embed.fields {
            b.field(field.name, field.value, field.inline);
        }

        if let Some(image) = embed.image {
            b.image(&image.url);
        }

        if let Some(timestamp) = embed.timestamp {
            b.timestamp(timestamp);
        }

        if let Some(thumbnail) = embed.thumbnail {
            b.thumbnail(&thumbnail.url);
        }

        if let Some(url) = embed.url {
            b.url(&url);
        }

        if let Some(title) = embed.title {
            b.title(&title);
        }

        if let Some(footer) = embed.footer {
            b.footer(move |f| {
                f.text(&footer.text);

                if let Some(icon_url) = footer.icon_url {
                    f.icon_url(&icon_url);
                }

                f
            });
        }

        b
    }
}

/// A builder to create a fake [`Embed`] object's author, for use with the
/// [`CreateEmbed::author`] method.
///
/// Requires that you specify a [`name`].
///
/// [`Embed`]: ../model/channel/struct.Embed.html
/// [`CreateEmbed::author`]: struct.CreateEmbed.html#method.author
/// [`name`]: #method.name
#[derive(Clone, Debug, Default)]
pub struct CreateEmbedAuthor(pub VecMap<&'static str, Value>);

impl CreateEmbedAuthor {
    /// Set the URL of the author's icon.
    pub fn icon_url(&mut self, icon_url: &str) -> &mut Self {
        self.0.insert("icon_url", Value::String(icon_url.to_string()));
        self
    }

    /// Set the author's name.
    pub fn name(&mut self, name: &str) -> &mut Self {
        self.0.insert("name", Value::String(name.to_string()));
        self
    }

    /// Set the author's URL.
    pub fn url(&mut self, url: &str) -> &mut Self {
        self.0.insert("url", Value::String(url.to_string()));
        self
    }

    pub fn build(&mut self) {}
}

/// A builder to create a fake [`Embed`] object's footer, for use with the
/// [`CreateEmbed::footer`] method.
///
/// This does not require any field be set.
///
/// [`Embed`]: ../model/channel/struct.Embed.html
/// [`CreateEmbed::footer`]: struct.CreateEmbed.html#method.footer
#[derive(Clone, Debug, Default)]
pub struct CreateEmbedFooter(pub VecMap<&'static str, Value>);

impl CreateEmbedFooter {
    /// Set the icon URL's value. This only supports HTTP(S).
    pub fn icon_url(&mut self, icon_url: &str) -> &mut Self {
        self.0.insert("icon_url", Value::String(icon_url.to_string()));
        self
    }

    /// Set the footer's text.
    pub fn text<D: Display>(&mut self, text: D) -> &mut Self {
        self.0.insert("text", Value::String(text.to_string()));
        self
    }
}

#[derive(Clone, Debug)]
pub struct Timestamp {
    pub ts: String,
}

impl From<String> for Timestamp {
    fn from(ts: String) -> Self {
        Self {
            ts: ts,
        }
    }
}

impl<'a> From<&'a str> for Timestamp {
    fn from(ts: &'a str) -> Self {
        Self {
            ts: ts.to_string(),
        }
    }
}

impl<'a, Tz: TimeZone> From<&'a DateTime<Tz>> for Timestamp
    where Tz::Offset: Display {
    fn from(dt: &'a DateTime<Tz>) -> Self {
        Self {
            ts: dt.to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod test {
    use model::channel::{Embed, EmbedField, EmbedFooter, EmbedImage, EmbedVideo};
    use serde_json::Value;
    use super::CreateEmbed;
    use utils::{self, Colour};

    #[test]
    fn test_from_embed() {
        let embed = Embed {
            author: None,
            colour: Colour::new(0xFF0011),
            description: Some("This is a test description".to_string()),
            fields: vec![
                EmbedField {
                    inline: false,
                    name: "a".to_string(),
                    value: "b".to_string(),
                },
                EmbedField {
                    inline: true,
                    name: "c".to_string(),
                    value: "z".to_string(),
                },
            ],
            footer: Some(EmbedFooter {
                icon_url: Some("https://i.imgur.com/XfWpfCV.gif".to_string()),
                proxy_icon_url: None,
                text: "This is a hakase footer".to_string(),
            }),
            image: Some(EmbedImage {
                height: 213,
                proxy_url: "a".to_string(),
                url: "https://i.imgur.com/XfWpfCV.gif".to_string(),
                width: 224,
            }),
            kind: "rich".to_string(),
            provider: None,
            thumbnail: None,
            timestamp: None,
            title: Some("hakase".to_string()),
            url: Some("https://i.imgur.com/XfWpfCV.gif".to_string()),
            video: Some(EmbedVideo {
                height: 213,
                url: "https://i.imgur.com/XfWpfCV.mp4".to_string(),
                width: 224,
            }),
        };

        let mut builder = CreateEmbed::from(embed);
            builder.colour(0xFF0011);
            builder.description("This is a hakase description");
            builder.image("https://i.imgur.com/XfWpfCV.gif");
            builder.title("still a hakase");
            builder.url("https://i.imgur.com/XfWpfCV.gif");

        let built = Value::Object(utils::vecmap_to_json_map(builder.0));

        let obj = json!({
            "color": 0xFF0011,
            "description": "This is a hakase description",
            "title": "still a hakase",
            "type": "rich",
            "url": "https://i.imgur.com/XfWpfCV.gif",
            "fields": [
                {
                    "inline": false,
                    "name": "a",
                    "value": "b",
                },
                {
                    "inline": true,
                    "name": "c",
                    "value": "z",
                },
            ],
            "image": {
                "url": "https://i.imgur.com/XfWpfCV.gif",
            },
            "footer": {
                "text": "This is a hakase footer",
                "icon_url": "https://i.imgur.com/XfWpfCV.gif",
            }
        });

        assert_eq!(built, obj);
    }
}
