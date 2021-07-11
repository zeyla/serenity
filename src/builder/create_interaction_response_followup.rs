use std::collections::HashMap;

use serde_json::Value;

use super::{CreateAllowedMentions, CreateEmbed};
use crate::builder::CreateComponents;
use crate::model::interactions::InteractionApplicationCommandCallbackDataFlags;
use crate::{http::AttachmentType, utils};

#[derive(Clone, Debug, Default)]
pub struct CreateInteractionResponseFollowup<'a>(
    pub HashMap<&'static str, Value>,
    pub Vec<AttachmentType<'a>>,
);

impl<'a> CreateInteractionResponseFollowup<'a> {
    /// Set the content of the message.
    ///
    /// **Note**: Message contents must be under 2000 unicode code points.
    #[inline]
    pub fn content<D: ToString>(&mut self, content: D) -> &mut Self {
        self._content(content.to_string())
    }

    fn _content(&mut self, content: String) -> &mut Self {
        self.0.insert("content", Value::String(content));
        self
    }

    /// Override the default username of the webhook
    #[inline]
    pub fn username<D: ToString>(&mut self, username: D) -> &mut Self {
        self._username(username.to_string())
    }

    fn _username(&mut self, username: String) -> &mut Self {
        self.0.insert("username", Value::String(username));
        self
    }

    /// Override the default avatar of the webhook
    #[inline]
    pub fn avatar<D: ToString>(&mut self, avatar_url: D) -> &mut Self {
        self._avatar(avatar_url.to_string())
    }

    fn _avatar(&mut self, avatar_url: String) -> &mut Self {
        self.0.insert("avatar_url", Value::String(avatar_url));
        self
    }
    /// Set whether the message is text-to-speech.
    ///
    /// Think carefully before setting this to `true`.
    ///
    /// Defaults to `false`.
    pub fn tts(&mut self, tts: bool) -> &mut Self {
        self.0.insert("tts", Value::Bool(tts));
        self
    }

    /// Appends a file to the message.
    pub fn add_file<T: Into<AttachmentType<'a>>>(&mut self, file: T) -> &mut Self {
        self.1.push(file.into());
        self
    }

    /// Appends a list of files to the message.
    pub fn add_files<T: Into<AttachmentType<'a>>, It: IntoIterator<Item = T>>(
        &mut self,
        files: It,
    ) -> &mut Self {
        self.1.extend(files.into_iter().map(|f| f.into()));
        self
    }

    /// Sets a list of files to include in the message.
    ///
    /// Calling this multiple times will overwrite the file list.
    /// To append files, call [`Self::add_file`] or [`Self::add_files`] instead.
    pub fn files<T: Into<AttachmentType<'a>>, It: IntoIterator<Item = T>>(
        &mut self,
        files: It,
    ) -> &mut Self {
        self.1 = files.into_iter().map(|f| f.into()).collect();
        self
    }

    /// Create an embed for the message.
    pub fn create_embed<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut CreateEmbed) -> &mut CreateEmbed,
    {
        let mut embed = CreateEmbed::default();
        f(&mut embed);
        self.add_embed(embed)
    }

    /// Adds an embed to the message.
    pub fn add_embed(&mut self, embed: CreateEmbed) -> &mut Self {
        let map = utils::hashmap_to_json_map(embed.0);
        let embed = Value::Object(map);

        self.0
            .entry("embeds")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .unwrap()
            .push(embed);

        self
    }

    /// Sets a list of embeds to include in the message.
    ///
    /// Calling this multiple times will overwrite the embed list.
    /// To append embeds, call [`Self::add_embed`] instead.
    pub fn embeds(&mut self, embeds: impl IntoIterator<Item = CreateEmbed>) -> &mut Self {
        let embeds =
            embeds.into_iter().map(|embed| utils::hashmap_to_json_map(embed.0).into()).collect();

        self.0.insert("embeds", Value::Array(embeds));
        self
    }

    /// Set the allowed mentions for the message.
    pub fn allowed_mentions<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut CreateAllowedMentions) -> &mut CreateAllowedMentions,
    {
        let mut allowed_mentions = CreateAllowedMentions::default();
        f(&mut allowed_mentions);
        let map = utils::hashmap_to_json_map(allowed_mentions.0);
        let allowed_mentions = Value::Object(map);

        self.0.insert("allowed_mentions", allowed_mentions);
        self
    }

    /// Sets the flags for the response.
    pub fn flags(&mut self, flags: InteractionApplicationCommandCallbackDataFlags) -> &mut Self {
        self.0.insert("flags", Value::Number(serde_json::Number::from(flags.bits())));
        self
    }

    /// Create components for this message.
    #[cfg(feature = "unstable_discord_api")]
    pub fn components<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut CreateComponents) -> &mut CreateComponents,
    {
        let mut components = CreateComponents::default();
        f(&mut components);

        self.0.insert("components", Value::Array(components.0));
        self
    }

    /// Sets the components of this message.
    #[cfg(feature = "unstable_discord_api")]
    pub fn set_components(&mut self, components: CreateComponents) -> &mut Self
    {
        self.0.insert("components", Value::Array(components.0));
        self
    }
}
