//! Tests for parsing Spigot-specific console output

use crate::parse::ConsoleMsgSpecific;
use crate::parse::ConsoleMsgType;
use chrono::Timelike;

#[test]
fn loading_libraries() {
    // spigot prints this non-standard line without a timestamp
    let msg = "Loading libraries, please wait...";
    assert!(ConsoleMsgSpecific::try_parse_from(msg).is_none());
}

#[test]
fn player_login() {
    let msg = "[23:11:12] [Server thread/INFO]: Cldfire[/127.0.0.1:56538] logged in with entity id 97 \
        at ([world]8185.897723692287, 65.0, -330.1145592972985)";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerLogin { generic_msg, name, ip, entity_id, coords, world } => {
            assert_eq!(generic_msg.msg, "Cldfire[/127.0.0.1:56538] logged in with entity id 97 \
                at ([world]8185.897723692287, 65.0, -330.1145592972985)");

            assert_eq!(name, "Cldfire");
            assert_eq!(ip, "127.0.0.1:56538");
            assert_eq!(entity_id, 97);
            assert_eq!(coords, (8185.897723692287, 65.0, -330.1145592972985));
            assert_eq!(world.unwrap(), "world");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_msg() {
    let msg = "[23:12:39] [Async Chat Thread - #8/INFO]: <Cldfire> hi!";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerMsg { generic_msg, name, msg } => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 12);
            assert_eq!(generic_msg.timestamp.second(), 39);
            assert_eq!(generic_msg.thread_name, "Async Chat Thread - #8");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "<Cldfire> hi!");

            assert_eq!(name, "Cldfire");
            assert_eq!(msg, "hi!");
        }
        _ => panic!("wrong variant")
    }
}