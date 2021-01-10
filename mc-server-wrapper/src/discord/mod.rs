use log::{debug, info, warn};

use twilight_cache_inmemory::{model::CachedMember, InMemoryCache, ResourceType};
use twilight_command_parser::{Command, CommandParserConfig, Parser};
use twilight_gateway::{Cluster, Event};
use twilight_http::{
    request::prelude::create_message::CreateMessageError, Client as DiscordClient,
};
use twilight_model::{
    channel::{message::MessageType, Message},
    gateway::{
        payload::{RequestGuildMembers, UpdateStatus},
        presence::Status,
        Intents,
    },
    id::{ChannelId, GuildId, UserId},
};

use mc_server_wrapper_lib::{communication::*, parse::*};
use minecraft_protocol::chat::{Color, Payload};

use util::{
    activity, format_mentions_in, format_online_players, tellraw_prefix, OnlinePlayerFormat,
};

use crate::ONLINE_PLAYERS;
use futures::{future, StreamExt};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc::Sender;

mod message_span_iter;
pub mod util;

static CHAT_PREFIX: &str = "[D] ";

/// Sets up a `DiscordBridge` and starts handling events
///
/// If `allow_status_updates` is set to `false` any calls to `update_status()`
/// will be no-ops
pub async fn setup_discord(
    token: String,
    bridge_channel_id: ChannelId,
    mc_cmd_sender: Sender<ServerCommand>,
    allow_status_updates: bool,
) -> anyhow::Result<DiscordBridge> {
    info!("Setting up Discord");
    let discord = DiscordBridge::new(token, bridge_channel_id, allow_status_updates).await?;

    let discord_clone = discord.clone();
    tokio::spawn(async move {
        let discord = discord_clone;
        let cmd_parser = DiscordBridge::command_parser();

        // For all received Discord events, map the event to a `ServerCommand`
        // (if necessary) and send it to the Minecraft server
        // TODO: don't unwrap here
        let mut events = discord.cluster_ref().unwrap().events();
        while let Some(e) = events.next().await {
            let discord = discord.clone();
            let cmd_sender_clone = mc_cmd_sender.clone();
            let cmd_parser_clone = cmd_parser.clone();

            // Update the cache
            discord.inner.as_ref().unwrap().cache.update(&e.1);

            tokio::spawn(async move {
                if let Err(e) = discord
                    .handle_discord_event(e, cmd_parser_clone, cmd_sender_clone)
                    .await
                {
                    warn!("Failed to handle Discord event: {}", e);
                }
            });
        }
    });

    Ok(discord)
}

/// Represents a maybe-present Discord bridge to a single text channel
///
/// All operations are no-ops if this struct is constructed without the
/// necessary info to set up the Discord bridge.
///
/// This struct can be cloned and passed around as needed.
#[derive(Debug, Clone)]
pub struct DiscordBridge {
    inner: Option<DiscordBridgeInner>,
    /// The ID of the channel we're bridging to
    bridge_channel_id: ChannelId,
    /// If set to `false` calls to `update_status()` will be no-ops
    allow_status_updates: bool,
}

// Groups together optionally-present things
//
// Everything in here can be trivially cloned
#[derive(Debug, Clone)]
struct DiscordBridgeInner {
    client: DiscordClient,
    cluster: Cluster,
    cache: InMemoryCache,
}

impl DiscordBridge {
    /// Connects to Discord with the given `token` and `bridge_channel_id`
    ///
    /// If `allow_status_updates` is set to `false` any calls to `update_status()`
    /// will be no-ops
    pub async fn new(
        token: String,
        bridge_channel_id: ChannelId,
        allow_status_updates: bool,
    ) -> anyhow::Result<Self> {
        let client = DiscordClient::new(&token);
        let cluster = Cluster::builder(
            &token,
            Intents::GUILDS | Intents::GUILD_MESSAGES | Intents::GUILD_MEMBERS,
        )
        .build()
        .await?;

        let cluster_spawn = cluster.clone();
        tokio::spawn(async move {
            cluster_spawn.up().await;
        });

        let cache = InMemoryCache::builder()
            .resource_types(ResourceType::GUILD | ResourceType::CHANNEL | ResourceType::MEMBER)
            .build();

        Ok(Self {
            inner: Some(DiscordBridgeInner {
                client,
                cluster,
                cache,
            }),
            bridge_channel_id,
            allow_status_updates,
        })
    }

    /// Constructs an instance of this struct that does nothing
    pub fn new_noop() -> Self {
        Self {
            inner: None,
            bridge_channel_id: ChannelId(0),
            allow_status_updates: false,
        }
    }

    /// Provides access to the `Cluster` inside this struct
    pub fn cluster(&self) -> Option<Cluster> {
        self.inner.as_ref().map(|i| i.cluster.clone())
    }

    /// Provides a reference to the `Cluster` inside this struct
    pub fn cluster_ref(&self) -> Option<&Cluster> {
        self.inner.as_ref().map(|i| &i.cluster)
    }

    /// Provides access to the `InMemoryCache` inside this struct
    pub fn cache(&self) -> Option<InMemoryCache> {
        self.inner.as_ref().map(|i| i.cache.clone())
    }

    /// Constructs a command parser for Discord commands
    pub fn command_parser<'a>() -> Parser<'a> {
        let mut config = CommandParserConfig::new();

        config.add_command("list", false);

        // TODO: make this configurable
        config.add_prefix("!mc ");

        Parser::new(config)
    }

    /// Get cached info for the guild member specified by the given IDs
    ///
    /// `None` will be returned if the member is not present in the cache.
    //
    // TODO: previously this was used to try fetching the member info over the HTTP
    // api if the member was not cached. Unfortunately support for caching out-of-band
    // like that was removed from twilight's inmemory cache crate, so we can't do that
    // any more.
    //
    // This method just exists to log failures to find requested member info in the
    // cache for now.
    pub async fn obtain_guild_member(
        &self,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Option<Arc<CachedMember>> {
        // First check the cache
        if let Some(cached_member) = self.cache().unwrap().member(guild_id, user_id) {
            Some(cached_member)
        } else {
            warn!(
                "Member info for user with guild_id {} and user_id {} was not cached",
                guild_id, user_id
            );

            None
        }
    }

    /// Handle an event from Discord
    ///
    /// The provided `cmd_parser` is used to parse commands (not
    /// `ServerCommands`) from Discord messages.
    #[allow(clippy::single_match)]
    pub async fn handle_discord_event(
        &self,
        event: (u64, Event),
        cmd_parser: Parser<'_>,
        mc_cmd_sender: Sender<ServerCommand>,
    ) -> anyhow::Result<()> {
        match event {
            (_, Event::Ready(_)) => {
                info!("Discord bridge online");
            }
            (shard_id, Event::GuildCreate(guild)) => {
                // Log the name of the channel we're bridging to as well if it's
                // in this guild
                if let Some(channel_name) = guild
                    .channels
                    .iter()
                    .find(|c| c.id() == self.bridge_channel_id)
                    .map(|c| c.name())
                {
                    info!(
                        "Connected to guild '{}', bridging chat to '#{}'",
                        guild.name, channel_name
                    );

                    // This is the guild containing the channel we're bridging to. We want to
                    // initially cache all of the members in the guild so that we can later use
                    // the cached info to display nicknames when outputting Discord messages in
                    // Minecraft
                    // TODO: if bigger servers start using this it might be undesirable to cache
                    // all member info right out of the gate
                    self.cluster()
                        .unwrap()
                        .command(
                            shard_id,
                            &RequestGuildMembers::builder(guild.id).query("", None),
                        )
                        .await?;
                } else {
                    info!("Connected to guild '{}'", guild.name);
                }
            }
            (_, Event::MessageCreate(msg)) => {
                if msg.kind == MessageType::Regular
                    && !msg.author.bot
                    && msg.channel_id == self.bridge_channel_id
                {
                    let cached_member = self
                        .obtain_guild_member(msg.guild_id.unwrap_or(GuildId(0)), msg.author.id)
                        .await;
                    let author_display_name = cached_member
                        .as_ref()
                        .and_then(|cm| cm.nick.as_ref())
                        .unwrap_or(&msg.author.name);

                    if let Some(command) = cmd_parser.parse(&msg.content) {
                        match command {
                            Command { name: "list", .. } => {
                                let response = {
                                    let online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
                                    format_online_players(
                                        &online_players,
                                        OnlinePlayerFormat::CommandResponse { short: false },
                                    )
                                };

                                self.clone().send_channel_msg(response);
                            }
                            _ => {}
                        }

                        return Ok(());
                    }

                    self.handle_attachments_in_msg(
                        &msg,
                        &author_display_name,
                        mc_cmd_sender.clone(),
                    )
                    .await;

                    self.handle_msg_content(&msg, &author_display_name, mc_cmd_sender.clone())
                        .await;

                    // We handle embeds after the message contents to replicate
                    // Discord's layout (embeds after message)
                    self.handle_embeds_in_msg(&msg, &author_display_name, mc_cmd_sender)
                        .await;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Handles any attachments in the given message
    async fn handle_attachments_in_msg(
        &self,
        msg: &Message,
        author_display_name: &str,
        mc_cmd_sender: Sender<ServerCommand>,
    ) {
        for attachment in &msg.attachments {
            let type_str = if attachment.height.is_some() {
                // TODO: it could also be a video....
                "image"
            } else {
                "file"
            };

            let tellraw_msg = tellraw_prefix()
                .then(Payload::text(&format!("{} uploaded ", author_display_name)))
                .italic(true)
                .color(Color::Gray)
                .then(Payload::text(type_str))
                .underlined(true)
                .italic(true)
                .color(Color::Gray)
                .hover_show_text(&format!(
                    "Click to open the {} in your web browser",
                    type_str
                ))
                .click_open_url(&attachment.url)
                .build();

            ConsoleMsg::new(
                ConsoleMsgType::Info,
                format!(
                    "{}{} uploaded {}: {}",
                    CHAT_PREFIX, author_display_name, type_str, attachment.url
                ),
            )
            .log();

            mc_cmd_sender
                .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
                .await
                .ok();
        }
    }

    /// Handles the content of the message
    ///
    /// This can only be called if `self.inner` is `Some`
    async fn handle_msg_content(
        &self,
        msg: &Message,
        author_display_name: &str,
        mc_cmd_sender: Sender<ServerCommand>,
    ) {
        if msg.content.is_empty() {
            debug!("Empty message from Discord: {:?}", &msg);
            return;
        }

        let cache = self.cache().unwrap();
        let guild_id = msg.guild_id.unwrap_or(GuildId(0));

        // Get info about mentioned members from the cache and / or API if available
        let cached_mentioned_members = future::join_all(
            msg.mentions
                .iter()
                .map(|m| m.id)
                .map(|id| async move { self.obtain_guild_member(guild_id, id).await }),
        )
        .await;

        // Use the cached info to format mentions with the member's nickname if one is
        // set
        let mut mentions_map = HashMap::new();
        for (mention, cmm) in msg.mentions.iter().zip(cached_mentioned_members.iter()) {
            mentions_map.insert(
                mention.id,
                cmm.as_ref()
                    .and_then(|cm| cm.nick.as_deref())
                    .unwrap_or_else(|| mention.name.as_str()),
            );
        }

        let tellraw_msg_builder = tellraw_prefix()
            .then(Payload::text(&format!("<{}> ", author_display_name)))
            .hover_show_text(&format!(
                "{}#{}",
                &msg.author.name, &msg.author.discriminator
            ));

        let (content, tellraw_msg_builder) = format_mentions_in(
            &msg.content,
            tellraw_msg_builder,
            mentions_map,
            &msg.mention_roles,
            cache.clone(),
        );

        // Tellraw commands do not get logged to the console, so we
        // make up for that here
        ConsoleMsg::new(
            ConsoleMsgType::Info,
            format!(
                "{}<{} ({}#{})> {}",
                CHAT_PREFIX,
                author_display_name,
                &msg.author.name,
                &msg.author.discriminator,
                &content
            ),
        )
        .log();

        mc_cmd_sender
            .send(ServerCommand::TellRawAll(
                tellraw_msg_builder.build().to_json().unwrap(),
            ))
            .await
            .ok();
    }

    /// Handles any embeds in the given message
    async fn handle_embeds_in_msg(
        &self,
        msg: &Message,
        author_display_name: &str,
        mc_cmd_sender: Sender<ServerCommand>,
    ) {
        for embed in msg.embeds.iter().filter(|e| e.url.is_some()) {
            // TODO: this is kinda ugly
            let link_text = if embed.title.is_some() && embed.provider.is_some() {
                if embed.provider.as_ref().unwrap().name.is_some() {
                    format!(
                        "{} - {}",
                        embed.provider.as_ref().unwrap().name.as_ref().unwrap(),
                        embed.title.as_ref().unwrap()
                    )
                } else {
                    embed.url.clone().unwrap()
                }
            } else {
                embed.url.clone().unwrap()
            };

            let tellraw_msg = tellraw_prefix()
                .then(Payload::text(&format!("{} linked \"", author_display_name)))
                .italic(true)
                .color(Color::Gray)
                .then(Payload::text(&link_text))
                .underlined(true)
                .italic(true)
                .color(Color::Gray)
                .hover_show_text("Click to open the link in your web browser")
                .click_open_url(embed.url.as_ref().unwrap())
                .then(Payload::text("\""))
                .italic(true)
                .color(Color::Gray)
                .build();

            ConsoleMsg::new(
                ConsoleMsgType::Info,
                format!(
                    "{}{} linked \"{}\": {}",
                    CHAT_PREFIX,
                    author_display_name,
                    link_text,
                    embed.url.as_ref().unwrap()
                ),
            )
            .log();

            mc_cmd_sender
                .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
                .await
                .ok();
        }
    }

    /// Sends the given text to the channel being bridged to
    ///
    /// A new task is spawned to send the message, and its `JoinHandle` is
    /// returned so its completion can be `await`ed if desired.
    pub fn send_channel_msg<T: Into<String> + Send + 'static>(
        self,
        text: T,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Some(inner) = self.inner {
                let content_res = inner
                    .client
                    .create_message(self.bridge_channel_id)
                    .content(text);

                match content_res {
                    Ok(cm) => {
                        if let Err(e) = cm.await {
                            warn!("Failed to send Discord message: {}", e);
                        }
                    }
                    Err(validation_err) => match validation_err {
                        CreateMessageError::ContentInvalid { content } => warn!(
                            "Attempted to send invalid message to Discord, content was: {}",
                            content
                        ),
                        _ => warn!(
                            "Attempted to send invalid message to Discord: {}",
                            validation_err
                        ),
                    },
                }
            }
        })
    }

    /// Sets the bot's status to the given text
    ///
    /// A new task is spawned to update the status, and its `JoinHandle` is
    /// returned so its completion can be `await`ed if desired.
    ///
    /// This will be a no-op if `self.allow_status_updates` is false
    pub fn update_status<T: Into<String> + Send + 'static>(
        self,
        text: T,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if !self.allow_status_updates {
                return;
            }
            let text = text.into();

            if let Some(inner) = self.inner {
                for shard_id in inner.cluster.info().keys() {
                    if let Some(shard) = inner.cluster.shard(*shard_id) {
                        match shard
                            .command(&UpdateStatus::new(
                                vec![activity(text.clone())],
                                false,
                                None,
                                Status::Online,
                            ))
                            .await
                        {
                            Ok(()) => {}
                            Err(e) => warn!("Failed to update bot's status: {}", e),
                        }
                    }
                }
            }
        })
    }
}
