//! Tests for parsing Spigot-specific console output

use crate::parse::{ConsoleMsg, ConsoleMsgSpecific};

#[test]
fn loading_libraries() {
    // spigot prints this non-standard line without a timestamp
    let msg = "Loading libraries, please wait...";
    assert!(ConsoleMsg::try_parse_from(msg).is_none());
}

#[test]
fn player_login() {
    let msg =
        "[23:11:12] [Server thread/INFO]: Cldfire[/127.0.0.1:56538] logged in with entity id 97 \
        at ([world]8185.897723692287, 65.0, -330.1145592972985)";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::PlayerLogin {
            name,
            ip,
            entity_id,
            coords,
            world,
        } => {
            assert_eq!(name, "Cldfire");
            assert_eq!(ip, "127.0.0.1:56538");
            assert_eq!(entity_id, 97);
            assert_eq!(coords, (8_185.898, 65.0, -330.114_56));
            assert_eq!(world.unwrap(), "world");
        }
        _ => unreachable!(),
    }
}

#[test]
fn player_msg() {
    let msg = "[23:12:39] [Async Chat Thread - #8/INFO]: <Cldfire> hi!";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::PlayerMsg { name, msg } => {
            assert_eq!(name, "Cldfire");
            assert_eq!(msg, "hi!");
        }
        _ => unreachable!(),
    }
}
