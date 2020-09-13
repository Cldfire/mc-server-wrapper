use super::CHAT_PREFIX;
use minecraft_protocol::chat::{Color, MessageBuilder, Payload};
use std::collections::{HashMap, HashSet};
use twilight_cache_inmemory::InMemoryCache;
use twilight_model::{
    gateway::presence::{Activity, ActivityType},
    id::{ChannelId, RoleId, UserId},
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
        created_at: None,
        details: None,
        flags: None,
        id: None,
        instance: None,
        kind: ActivityType::Playing,
        name,
        emoji: None,
        party: None,
        secrets: None,
        state: None,
        timestamps: None,
        url: None,
    }
}

/// Formats mentions in the given content using the given info
///
/// `mentions` maps mentioned user IDs to their names. It is your responsibility
/// to handle nicknames and pass them in appropriately if that's something you
/// care about.
///
/// The given `cache` is used to get data to replace channel and role mention
/// names with
// TODO: this code is complicated, explore strategies to simplify?
// TODO: does not handle escaped mentions (but this is a SUPER edge case and
// the Discord client doesn't even handle those right either)
pub fn format_mentions_in<S: Into<String>>(
    content: S,
    mentions: HashMap<UserId, &str>,
    mention_roles: &[RoleId],
    cache: InMemoryCache,
) -> String {
    enum MentionType {
        User,
        UserNickname,
        Channel,
        Role,
        Unknown,
    }
    let mut content = content.into();

    let mut content_slice = &content.clone()[..];
    while let Some(idx) = content_slice.find('<') {
        let mut mention_type = MentionType::Unknown;
        let mut possible_id = "";
        let mut closing_angle_idx = None;

        // Grabs the stuff between the start of a possible mention and the next
        // closing bracket
        let mut possible_id_after = |forward_idx: usize| {
            closing_angle_idx = if let Some(forward_slice) = content_slice.get(forward_idx..) {
                // We need to make sure we only look for a closing bracket after
                // the "mention-beginning" stuff we found, and not from the start
                // of the entire slice (there could be a closing bracket in front
                // of the "mention-beginning" stuff which is not what we would want
                // to find)
                //
                // If we do find an index to a closing bracket, we need to add
                // `forward_idx` to it since we are then using the found index
                // in the original slice
                forward_slice.find('>').map(|n| n + forward_idx)
            } else {
                None
            };

            if let Some(closing_angle_idx) = closing_angle_idx {
                possible_id = &content_slice[forward_idx..closing_angle_idx];
            }
        };

        // Check if this could be a mention
        if let Some(slice) = content_slice.get(idx..idx + 3) {
            // Note that the order of evaluation here matters
            if slice.starts_with("<@!") {
                mention_type = MentionType::UserNickname;
                possible_id_after(idx + 3);
            } else if slice.starts_with("<@&") {
                mention_type = MentionType::Role;
                possible_id_after(idx + 3);
            } else if slice.starts_with("<@") {
                // As far as I know it is correct to treat this as a user mention
                // TODO: once `strip_prefix` is stable, merge with the above branch?
                mention_type = MentionType::User;
                possible_id_after(idx + 2);
            } else if slice.starts_with("<#") {
                mention_type = MentionType::Channel;
                possible_id_after(idx + 2);
            }
        }

        // If it is a mention, replace it
        if let Ok(id) = possible_id.parse::<u64>() {
            let mut replace_mention = |with: &str| {
                let idx_diff = content.len() - content_slice.len();
                content.replace_range(
                    // `idx` is relative to the slice into the original string
                    // that we are adjusting the start of to proceed through
                    // the string
                    //
                    // in order to index back into the original string we have
                    // to offset the indices
                    idx + idx_diff..=closing_angle_idx.unwrap() + idx_diff,
                    with,
                );
            };

            match mention_type {
                // TODO: we should not format `MentionType::User` with the user's nick
                // ...but it looks like Discord doesn't really follow this convention so hm
                MentionType::User | MentionType::UserNickname => {
                    if let Some(name) = mentions
                        .iter()
                        .find(|(user_id, _)| id == user_id.0)
                        .map(|(_, name)| name)
                    {
                        replace_mention(&format!("@{}", name));
                    }
                }
                MentionType::Channel => {
                    if let Some(channel) = cache.guild_channel(ChannelId(id)) {
                        replace_mention(&format!("#{}", channel.name()));
                    }
                }
                MentionType::Role => {
                    if let Some(role_id) = mention_roles.iter().find(|r| id == r.0) {
                        if let Some(role) = cache.role(*role_id) {
                            replace_mention(&format!("@{}", &role.name));
                        }
                    }
                }
                MentionType::Unknown => log::warn!(
                    "Encountered unknown mention type in Discord message: {}",
                    &content
                ),
            }
        }

        content_slice = if let Some(end_idx) = closing_angle_idx {
            // If we found a closing angle bracket, either we found and handled a
            // mention in between or there was no valid mention in between. In
            // either case we can safely skip to it
            &content_slice[end_idx..]
        } else {
            // We didn't find a closing angle bracket, so we can't do anything
            // except skip to the next character (if one exists)
            if let Some(more) = content_slice.get(idx + 1..) {
                more
            } else {
                ""
            }
        };
    }

    content
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
    online_players: &HashSet<String>,
    format: OnlinePlayerFormat,
) -> String {
    // Sort the players for stable name order and sanitize their names
    let mut online_players_vec: Vec<_> = online_players.iter().map(sanitize_for_markdown).collect();
    online_players_vec.sort();

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
    use super::sanitize_for_markdown;

    #[test]
    fn sanitize_markdown() {
        let testcase = "~*`cdawg_m`>";
        assert_eq!(sanitize_for_markdown(testcase), "\\~\\*\\`cdawg\\_m\\`\\>");
    }

    mod content_format_mentions {
        use super::super::format_mentions_in;
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
                    name: "test-role".into(),
                    permissions: Permissions::empty(),
                    position: 0,
                },
            })
        }

        #[test]
        fn blank_message() {
            let msg = "";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, "");
        }

        #[test]
        fn fake_mention() {
            let msg = "the upcoming bracket <@thing is not a mention";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, msg);
        }

        #[test]
        fn closing_bracket_then_start_mention() {
            let msg = "><@!kksdk";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, msg);
        }

        #[test]
        fn fake_mention_crazy() {
            let msg = "<<><><@!><#><>#<>>>>";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, msg);
        }

        #[test]
        fn fake_mention_bad_id() {
            let msg = "<@!12notanumber>";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, msg);
        }

        #[test]
        fn one_mention_no_info() {
            let msg = "this has a mention: <@123>, but we're not passing mentions";
            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());

            assert_eq!(formatted, msg);
        }

        #[test]
        fn one_mention_with_info() {
            let msg = "this has a mention: <@123>, and we are passing mentions";
            let mut mentions = HashMap::new();
            mentions.insert(UserId(123), "TestName");

            let formatted = format_mentions_in(msg, mentions, &[], InMemoryCache::new());
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

            let formatted = format_mentions_in(msg, mentions, &[], InMemoryCache::new());
            assert_eq!(formatted, "@TestName, and even @AnotherTest!");
        }

        #[test]
        fn three_mentions_some_with_info() {
            let msg = "<@123>, and even <@!321>, and wow: <@3234>";
            let mut mentions = HashMap::new();
            mentions.insert(UserId(123), "TestName");
            mentions.insert(UserId(3234), "WowTest");

            let formatted = format_mentions_in(msg, mentions, &[], InMemoryCache::new());
            assert_eq!(formatted, "@TestName, and even <@!321>, and wow: @WowTest");
        }

        #[test]
        fn channel_mention_no_info() {
            let msg = "this is a channel mention: <#1234>";

            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());
            assert_eq!(formatted, msg);
        }

        #[test]
        fn channel_mention_with_info() {
            let msg = "this is a channel mention: <#1234>";

            let cache = InMemoryCache::new();
            cache.update(&make_text_channel());

            let formatted = format_mentions_in(msg, HashMap::new(), &[], cache);
            assert_eq!(formatted, "this is a channel mention: #test-channel");
        }

        #[test]
        fn channel_mention_with_others() {
            let msg = "<@1234> <#245> this is a channel mention: <#1234>";

            let cache = InMemoryCache::new();
            cache.update(&make_text_channel());

            let formatted = format_mentions_in(msg, HashMap::new(), &[], cache);
            assert_eq!(
                formatted,
                "<@1234> <#245> this is a channel mention: #test-channel"
            );
        }

        #[test]
        fn role_mention_no_info() {
            let msg = "this is a role mention: <@&2345>";

            let formatted = format_mentions_in(msg, HashMap::new(), &[], InMemoryCache::new());
            assert_eq!(formatted, "this is a role mention: <@&2345>");
        }

        #[test]
        fn role_mention_with_partial_info() {
            let msg = "this is a role mention: <@&2345>";

            let cache = InMemoryCache::new();
            cache.update(&make_role());

            let formatted = format_mentions_in(msg, HashMap::new(), &[], cache);
            assert_eq!(formatted, msg);
        }

        #[test]
        fn role_mention_with_info() {
            let msg = "this is a role mention: <@&2345>";

            let cache = InMemoryCache::new();
            cache.update(&make_role());

            let formatted = format_mentions_in(msg, HashMap::new(), &[RoleId(2345)], cache);
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

            let formatted = format_mentions_in(msg, mentions, &[RoleId(2345)], cache);
            assert_eq!(
                formatted,
                "@TestName this channel (#test-channel) is pretty cool for the role @test-role!"
            );
        }
    }

    mod format_online_players_command_response {
        use super::super::format_online_players;
        use std::collections::HashSet;

        mod common {
            use super::*;
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn markdown_in_names() {
                let mut online_players = HashSet::new();
                online_players.insert("p1_".into());
                online_players.insert("*`p2`".into());
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
                let online_players = HashSet::new();
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
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
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
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
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
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
                online_players.insert("p3".into());
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
            use super::*;
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn four_players() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
                online_players.insert("p3".into());
                online_players.insert("p4".into());
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
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p3".into());
                online_players.insert("p2".into());
                online_players.insert("p4".into());
                online_players.insert("p6".into());
                online_players.insert("p5".into());
                online_players.insert("p7".into());
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
            use super::*;
            use crate::discord::util::OnlinePlayerFormat;

            #[test]
            fn seven_players() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p3".into());
                online_players.insert("p2".into());
                online_players.insert("p4".into());
                online_players.insert("p6".into());
                online_players.insert("p5".into());
                online_players.insert("p7".into());
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
        use crate::discord::util::{format_online_players, OnlinePlayerFormat};
        use std::collections::HashSet;

        #[test]
        fn one_player() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1");
        }

        #[test]
        fn two_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1 and p2");
        }

        #[test]
        fn three_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1, p2, and p3");
        }

        #[test]
        fn four_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            online_players.insert("p4".into());
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with p1, p2, p3, and p4");
        }

        #[test]
        fn lots_of_players() {
            let mut online_players = HashSet::new();
            online_players.insert("player1".into());
            online_players.insert("player2".into());
            online_players.insert("player3".into());
            online_players.insert("player4".into());
            online_players.insert("player5".into());
            online_players.insert("player6".into());
            online_players.insert("player7".into());
            online_players.insert("player8".into());
            online_players.insert("player9".into());
            online_players.insert("player10".into());
            online_players.insert("player11".into());
            online_players.insert("player12".into());
            online_players.insert("player13".into());
            online_players.insert("player14".into());
            online_players.insert("player15".into());
            let formatted = format_online_players(&online_players, OnlinePlayerFormat::BotStatus);

            assert_eq!(&formatted, "Minecraft with player1, player10, player11, player12, player13, player14, player15, player2, and player3 (+ 6 more)");
            assert!(formatted.len() <= 128);
        }
    }
}
