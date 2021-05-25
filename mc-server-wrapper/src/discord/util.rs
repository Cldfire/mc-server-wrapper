use crate::OnlinePlayerInfo;

use super::{message_span_iter::MessageSpan, CHAT_PREFIX};
use minecraft_chat::{Color, MessageBuilder, Payload};
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
};
use twilight_cache_inmemory::InMemoryCache;
use twilight_mention::parse::MentionType;
use twilight_model::{
    gateway::presence::{Activity, ActivityType},
    id::{RoleId, UserId},
};

/// Returns a `MessageBuilder` with a nice prefix for Discord messages in
/// Minecraft
pub fn tellraw_prefix() -> MessageBuilder {
    // Setting styles on the first payload sets them for all future payloads
    // just fyi
    MessageBuilder::builder(Payload::text(""))
        .then(Payload::text(CHAT_PREFIX))
        .bold(true)
        .color(Color::LightPurple)
}

/// Helper to make a status for the bot
pub fn activity(name: String) -> Activity {
    Activity {
        application_id: None,
        assets: None,
        buttons: vec![],
        created_at: None,
        details: None,
        emoji: None,
        flags: None,
        id: None,
        instance: None,
        kind: ActivityType::Playing,
        name,
        party: None,
        secrets: None,
        state: None,
        timestamps: None,
        url: None,
    }
}

/// Formats mentions in the given content using the given info.
///
/// `mentions` maps mentioned user IDs to their names. It is your responsibility
/// to handle nicknames and pass them in appropriately if that's something you
/// care about.
///
/// The given `cache` is used to get data to replace channel and role mention
/// names with.
///
/// The given `message_builder` is used to build up a Minecraft chat object with
/// well-formatted text.
///
/// Returns (formatted_string, modified_chat_object_builder)
pub fn format_mentions_in<S: AsRef<str>>(
    content: S,
    mut message_builder: MessageBuilder,
    mentions: HashMap<UserId, &str>,
    mention_roles: &[RoleId],
    cache: InMemoryCache,
) -> (String, MessageBuilder) {
    // TODO: write a mc chat object crate to clean this code up
    let mut cows = vec![];

    for span in MessageSpan::iter(content.as_ref()) {
        // Map each span into well-formatted text, both for Minecraft and anything
        // else (like the TUI logs)
        match span {
            MessageSpan::Text(text) => {
                message_builder = message_builder.then(Payload::text(text));
                cows.push(Cow::from(text));
            }
            MessageSpan::Mention(mention_type, raw) => match mention_type {
                MentionType::Channel(id) => {
                    let cow = cache
                        .guild_channel(id)
                        .map(|channel| Cow::from(format!("#{}", channel.name())))
                        // Throughout this function we fallback to the raw, unformatted
                        // text if we're unable to fetch relevant info from the cache
                        .unwrap_or_else(|| Cow::from(raw));

                    message_builder = message_builder
                        .then(Payload::text(cow.as_ref()))
                        .color(Color::Blue);
                    cows.push(cow);
                }
                MentionType::Emoji(id) => {
                    // Non-custom emoji don't fall under this branch, but it would be
                    // annoying and non-performant to parse those out and replace them
                    // with :names:
                    let cow = cache
                        .emoji(id)
                        .map(|emoji| Cow::from(format!(":{}:", &emoji.name)))
                        .unwrap_or_else(|| Cow::from(raw));

                    message_builder = message_builder.then(Payload::text(cow.as_ref()));
                    cows.push(cow);
                }
                MentionType::Role(RoleId(id)) => {
                    let cow = mention_roles
                        .iter()
                        .find(|r| id == r.0)
                        .and_then(|role_id| cache.role(*role_id))
                        .map(|role| Cow::from(format!("@{}", &role.name)))
                        .unwrap_or_else(|| Cow::from(raw));

                    message_builder = message_builder
                        .then(Payload::text(cow.as_ref()))
                        .color(Color::Blue);
                    cows.push(cow)
                }
                MentionType::User(id) => {
                    let cow = mentions
                        .get(&id)
                        .map(|name| Cow::from(format!("@{}", name)))
                        .unwrap_or_else(|| Cow::from(raw));

                    message_builder = message_builder
                        .then(Payload::text(cow.as_ref()))
                        .color(Color::Blue);

                    if let Some(cached_user) = cache.user(id) {
                        message_builder = message_builder.hover_show_text(&format!(
                            "{}#{}",
                            &cached_user.name, &cached_user.discriminator
                        ));
                    }

                    cows.push(cow);
                }
                _ => {
                    message_builder = message_builder.then(Payload::text(raw));
                    cows.push(Cow::from(raw));
                }
            },
        }
    }

    (cows.into_iter().collect(), message_builder)
}

/// Different formats online player data can be turned into
#[derive(Debug)]
pub enum OnlinePlayerFormat {
    /// Format intended to be used as the response to a command
    CommandResponse {
        /// Setting this to `true` will truncate the list to 3 players
        short: bool,
    },
    /// Format intended to be used for a bot's status
    BotStatus,
}

/// Utility function to return a neatly formatted string describing who's
/// playing Minecraft
///
/// `short` can be set to true to truncate the list.
pub fn format_online_players(
    online_players: &BTreeMap<String, OnlinePlayerInfo>,
    format: OnlinePlayerFormat,
) -> String {
    // Sanitize player names if necessary
    // TODO: we don't need a vec here
    let online_players_vec: Vec<_> = online_players
        .keys()
        .map(|n| match format {
            OnlinePlayerFormat::BotStatus => n.clone(),
            OnlinePlayerFormat::CommandResponse { .. } => sanitize_for_markdown(n),
        })
        .collect();

    match format {
        OnlinePlayerFormat::CommandResponse { short } => match online_players.len() {
            0 => "Nobody is playing Minecraft".into(),
            1 => format!("{} is playing Minecraft", online_players_vec[0]),
            2 => format!(
                "{} and {} are playing Minecraft",
                online_players_vec[0], online_players_vec[1]
            ),
            _ => format!(
                "{} are playing Minecraft",
                online_players_list(&online_players_vec, short)
            ),
        },
        OnlinePlayerFormat::BotStatus => match online_players.len() {
            0 => "Minecraft with nobody".into(),
            1 => format!("Minecraft with {}", online_players_vec[0]),
            2 => format!(
                "Minecraft with {} and {}",
                online_players_vec[0], online_players_vec[1]
            ),
            _ => {
                // The character limit for the bot's status message appears to be 128
                // characters. Our approach here is to display as many full online
                // player names as possible and then resort to a (+ __ more) at the end
                // for any names that won't fit in the character limit
                let mut string = String::with_capacity(128);
                string.push_str("Minecraft with ");

                let mut i = 0;
                for name in &online_players_vec {
                    // We're nearing the max character limit. We need room for:
                    //
                    // and ________________ (+ ___ more)
                    //
                    // which is for:
                    //
                    // * the last player name (max length: 16 chars)
                    // * the "+ x more" text (max length: 3 chars, because it's not
                    //     physically possible to have >999 players on a single MC
                    //     server instance)
                    //
                    // This totals 33 chars of space we need to reserve. There is
                    // a *ton* of room to be smarter here, but like... effort.
                    if string.len() + name.len() + 2 > 95 {
                        if i == online_players_vec.len() - 1 {
                            // If we only have one name left then we'll just add it
                        } else {
                            break;
                        }
                    }

                    if i == online_players_vec.len() - 1 {
                        string.push_str("and ");
                        string.push_str(name);
                    } else {
                        string.push_str(name);
                        string.push_str(", ");
                    }

                    // Keep track of how many full names we have included in the string
                    i += 1;
                }

                if i < online_players_vec.len() {
                    // We need the (+ ___ more) piece
                    //
                    // Note that we have at least two names left when we've gotten here
                    string.push_str("and ");
                    string.push_str(&online_players_vec[i]);
                    i += 1;

                    string.push_str(&format!(" (+ {} more)", online_players_vec.len() - i));
                }

                string
            }
        },
    }
}

/// Formats a sorted array of online player names into a neat list
fn online_players_list(online_players: &[String], short: bool) -> String {
    if short {
        let mut string = format!(
            "{}, {}, and {}",
            online_players[0], online_players[1], online_players[2]
        );

        if online_players.len() > 3 {
            string.push_str(&format!(" (+ {} more)", online_players.len() - 3));
        }

        string
    } else {
        let mut string = String::new();

        for player in online_players[..online_players.len() - 1].iter() {
            string.push_str(&format!("{}, ", player));
        }

        string.push_str(&format!("and {}", online_players.last().unwrap()));
        string
    }
}

/// Sanitizes the given text for usage in a markdown context
pub fn sanitize_for_markdown<T: AsRef<str>>(text: T) -> String {
    let text = text.as_ref();

    text.chars().fold(String::new(), |mut s, c| {
        match c {
            '*' | '_' | '~' | '>' | '`' => {
                s.push('\\');
                s.push(c);
            }
            _ => s.push(c),
        }
        s
    })
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use crate::OnlinePlayerInfo;

    use super::sanitize_for_markdown;

    #[test]
    fn sanitize_markdown() {
        let testcase = "~*`cdawg_m`>";
        assert_eq!(sanitize_for_markdown(testcase), "\\~\\*\\`cdawg\\_m\\`\\>");
    }

    mod content_format_mentions {
        use super::super::format_mentions_in;
        use minecraft_chat::{MessageBuilder, Payload};
        use std::collections::HashMap;
        use twilight_cache_inmemory::InMemoryCache;
        use twilight_model::{
            channel::{Channel, ChannelType, GuildChannel, TextChannel},
            gateway::{event::Event, payload},
            guild::{Permissions, Role},
            id::{ChannelId, GuildId, RoleId, UserId},
        };

        fn make_text_channel() -> Event {
            Event::ChannelCreate(payload::ChannelCreate(Channel::Guild(GuildChannel::Text(
                TextChannel {
                    id: ChannelId(1234),
                    guild_id: Some(GuildId(0)),
                    kind: ChannelType::GuildText,
                    last_message_id: None,
                    last_pin_timestamp: None,
                    name: "test-channel".into(),
                    nsfw: false,
                    permission_overwrites: vec![],
                    parent_id: None,
                    position: 0,
                    rate_limit_per_user: None,
                    topic: Some("a test channel".into()),
                },
            ))))
        }

        fn make_role() -> Event {
            Event::RoleCreate(payload::RoleCreate {
                guild_id: GuildId(0),
                role: Role {
                    id: RoleId(2345),
                    color: 0,
                    hoist: false,
                    managed: false,
                    mentionable: true,
                    tags: None,
                    name: "test-role".into(),
                    permissions: Permissions::empty(),
                    position: 0,
                },
            })
        }

        #[test]
        fn blank_message() {
            let msg = "";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, "");
        }

        #[test]
        fn fake_mention() {
            let msg = "the upcoming bracket <@thing is not a mention";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, msg);
        }

        #[test]
        fn closing_bracket_then_start_mention() {
            let msg = "><@!kksdk";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, msg);
        }

        #[test]
        fn fake_mention_crazy() {
            let msg = "<<><><@!><#><>#<>>>>";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, msg);
        }

        #[test]
        fn fake_mention_bad_id() {
            let msg = "<@!12notanumber>";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, msg);
        }

        #[test]
        fn one_mention_no_info() {
            let msg = "this has a mention: <@123>, but we're not passing mentions";
            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );

            assert_eq!(formatted, msg);
        }

        #[test]
        fn one_mention_with_info() {
            let msg = "this has a mention: <@123>, and we are passing mentions";
            let mut mentions = HashMap::new();
            mentions.insert(UserId(123), "TestName");

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                mentions,
                &[],
                InMemoryCache::new(),
            );
            assert_eq!(
                formatted,
                "this has a mention: @TestName, and we are passing mentions"
            );
        }

        #[test]
        fn two_mentions_with_info() {
            let msg = "<@123>, and even <@!321>!";
            let mut mentions = HashMap::new();
            mentions.insert(UserId(123), "TestName");
            mentions.insert(UserId(321), "AnotherTest");

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                mentions,
                &[],
                InMemoryCache::new(),
            );
            assert_eq!(formatted, "@TestName, and even @AnotherTest!");
        }

        #[test]
        fn three_mentions_some_with_info() {
            let msg = "<@123>, and even <@!321>, and wow: <@3234>";
            let mut mentions = HashMap::new();
            mentions.insert(UserId(123), "TestName");
            mentions.insert(UserId(3234), "WowTest");

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                mentions,
                &[],
                InMemoryCache::new(),
            );
            assert_eq!(formatted, "@TestName, and even <@!321>, and wow: @WowTest");
        }

        #[test]
        fn channel_mention_no_info() {
            let msg = "this is a channel mention: <#1234>";

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );
            assert_eq!(formatted, msg);
        }

        #[test]
        fn channel_mention_with_info() {
            let msg = "this is a channel mention: <#1234>";

            let cache = InMemoryCache::new();
            cache.update(&make_text_channel());

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                cache,
            );
            assert_eq!(formatted, "this is a channel mention: #test-channel");
        }

        #[test]
        fn channel_mention_with_others() {
            let msg = "<@1234> <#245> this is a channel mention: <#1234>";

            let cache = InMemoryCache::new();
            cache.update(&make_text_channel());

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                cache,
            );
            assert_eq!(
                formatted,
                "<@1234> <#245> this is a channel mention: #test-channel"
            );
        }

        #[test]
        fn role_mention_no_info() {
            let msg = "this is a role mention: <@&2345>";

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                InMemoryCache::new(),
            );
            assert_eq!(formatted, "this is a role mention: <@&2345>");
        }

        #[test]
        fn role_mention_with_partial_info() {
            let msg = "this is a role mention: <@&2345>";

            let cache = InMemoryCache::new();
            cache.update(&make_role());

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[],
                cache,
            );
            assert_eq!(formatted, msg);
        }

        #[test]
        fn role_mention_with_info() {
            let msg = "this is a role mention: <@&2345>";

            let cache = InMemoryCache::new();
            cache.update(&make_role());

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                HashMap::new(),
                &[RoleId(2345)],
                cache,
            );
            assert_eq!(formatted, "this is a role mention: @test-role");
        }

        #[test]
        fn all_combined() {
            let msg = "<@1212> this channel (<#1234>) is pretty cool for the role <@&2345>!";

            let mut mentions = HashMap::new();
            mentions.insert(UserId(1212), "TestName");

            let cache = InMemoryCache::new();
            cache.update(&make_role());
            cache.update(&make_text_channel());

            let (formatted, _) = format_mentions_in(
                msg,
                MessageBuilder::builder(Payload::text("")),
                mentions,
                &[RoleId(2345)],
                cache,
            );
            assert_eq!(
                formatted,
                "@TestName this channel (#test-channel) is pretty cool for the role @test-role!"
            );
        }
    }

    fn make_players_map<'a>(
        names: impl IntoIterator<Item = &'a &'a str>,
    ) -> BTreeMap<String, OnlinePlayerInfo> {
        let mut online_players = BTreeMap::new();
        names.into_iter().for_each(|n| {
            online_players.insert(n.to_string(), OnlinePlayerInfo::default());
        });

        online_players
    }

    mod format_online_players_command_response {
        use super::super::format_online_players;

        mod common {
            use super::{super::make_players_map, *};
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn markdown_in_names() {
                let online_players = make_players_map(&["p1_", "*`p2`"]);
                let expected = "\\*\\`p2\\` and p1\\_ are playing Minecraft";

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn no_players() {
                let online_players = make_players_map(&[]);
                let expected = "Nobody is playing Minecraft";

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn one_player() {
                let online_players = make_players_map(&["p1"]);
                let expected = "p1 is playing Minecraft";

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn two_players() {
                let online_players = make_players_map(&["p1", "p2"]);
                let expected = "p1 and p2 are playing Minecraft";

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn three_players() {
                let online_players = make_players_map(&["p1", "p2", "p3"]);
                let expected = "p1, p2, and p3 are playing Minecraft";

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );
                assert_eq!(&formatted, expected);
            }
        }

        mod short {
            use super::{super::make_players_map, *};
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn four_players() {
                let online_players = make_players_map(&["p1", "p2", "p3", "p4"]);
                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );

                assert_eq!(
                    &formatted,
                    "p1, p2, and p3 (+ 1 more) are playing Minecraft"
                );
            }

            #[test]
            fn seven_players() {
                let online_players = make_players_map(&["p1", "p3", "p2", "p4", "p6", "p5", "p7"]);
                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: true },
                );

                assert_eq!(
                    &formatted,
                    "p1, p2, and p3 (+ 4 more) are playing Minecraft"
                );
            }
        }

        mod long {
            use super::{super::make_players_map, *};
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn seven_players() {
                let online_players = make_players_map(&["p1", "p2", "p3", "p4", "p5", "p6", "p7"]);
                let formatted = format_online_players(
                    &online_players,
                    OnlinePlayerFormat::CommandResponse { short: false },
                );

                assert_eq!(
                    &formatted,
                    "p1, p2, p3, p4, p5, p6, and p7 are playing Minecraft"
                );
            }
        }
    }

    mod format_online_players_bot_status {
        use super::make_players_map;
        use crate::discord::util::{format_online_players, OnlinePlayerFormat};

        #[test]
        fn one_player() {
            let online_players = make_players_map(&["p1"]);
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1");
        }

        #[test]
        fn two_players() {
            let online_players = make_players_map(&["p1", "p2"]);
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1 and p2");
        }

        #[test]
        fn three_players() {
            let online_players = make_players_map(&["p1", "p2", "p3"]);
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1, p2, and p3");
        }

        #[test]
        fn four_players() {
            let online_players = make_players_map(&["p1", "p2", "p3", "p4"]);
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1, p2, p3, and p4");
        }

        #[test]
        fn lots_of_players() {
            let online_players = make_players_map(&[
                "player1", "player2", "player3", "player11", "player5", "player6", "player7",
                "player8", "player9", "player10", "player4", "player12", "player13", "player14",
                "player15",
            ]);
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with player1, player10, player11, player12, player13, player14, player15, player2, and player3 (+ 6 more)");
            assert!(formatted.len() <= 128);
        }
    }
}
