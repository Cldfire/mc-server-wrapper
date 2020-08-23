//! Tests for parsing vanilla console output

use crate::parse::{ConsoleMsg, ConsoleMsgSpecific, ConsoleMsgType};
use chrono::Timelike;

#[test]
fn warn_msg() {
    let msg = "[23:10:30] [main/WARN]: Ambiguity between arguments [teleport, targets, location] \
        and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]";
    let console_msg = ConsoleMsg::try_parse_from(msg).unwrap();

    assert_eq!(console_msg.timestamp.hour(), 23);
    assert_eq!(console_msg.timestamp.minute(), 10);
    assert_eq!(console_msg.timestamp.second(), 30);
    assert_eq!(console_msg.thread_name, "main");
    assert_eq!(console_msg.msg_type, ConsoleMsgType::Warn);
    assert_eq!(
        console_msg.msg,
        "Ambiguity between arguments [teleport, targets, location] \
        and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]"
    );

    assert!(ConsoleMsgSpecific::try_parse_from(&console_msg).is_none());
}

#[test]
fn info_msg() {
    let msg = "[23:10:31] [Server thread/INFO]: Starting Minecraft server on *:25565";
    let console_msg = ConsoleMsg::try_parse_from(msg).unwrap();

    assert_eq!(console_msg.timestamp.hour(), 23);
    assert_eq!(console_msg.timestamp.minute(), 10);
    assert_eq!(console_msg.timestamp.second(), 31);
    assert_eq!(console_msg.thread_name, "Server thread");
    assert_eq!(console_msg.msg_type, ConsoleMsgType::Info);
    assert_eq!(console_msg.msg, "Starting Minecraft server on *:25565");

    assert!(ConsoleMsgSpecific::try_parse_from(&console_msg).is_none());
}

#[test]
fn newline() {
    let msg = "\n";
    assert!(ConsoleMsg::try_parse_from(msg).is_none());
}

#[test]
fn blank_here() {
    // somehow occurs when rapidly firing unknown commands
    let msg = "[19:23:04] [Server thread/INFO]: <--[HERE]";
    let console_msg = ConsoleMsg::try_parse_from(msg).unwrap();

    assert!(ConsoleMsgSpecific::try_parse_from(&console_msg).is_none());
}

#[test]
fn must_accept_eula() {
    let msg = "[00:03:56] [Server thread/INFO]: You need to agree to the EULA in order to run the \
        server. Go to eula.txt for more info.";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    assert_eq!(specific_msg, ConsoleMsgSpecific::MustAcceptEula);
}

#[test]
fn player_msg() {
    let msg = "[23:12:39] [Server thread/INFO]: <Cldfire> hi!";
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

#[test]
fn player_login() {
    let msg = "[23:11:12] [Server thread/INFO]: Cldfire[/127.0.0.1:56538] logged in with entity \
        id 121 at (-2.5, 63.0, 256.5)";
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
            assert_eq!(entity_id, 121);
            assert_eq!(coords, (-2.5, 63.0, 256.5));
            assert!(world.is_none());
        }
        _ => unreachable!(),
    }
}

#[test]
fn player_auth() {
    let msg = "[23:11:12] [User Authenticator #1/INFO]: UUID of player Cldfire is \
        361e5fb3-dbce-4f91-86b2-43423a4888d5";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::PlayerAuth { name, uuid } => {
            assert_eq!(name, "Cldfire");
            assert_eq!(uuid, "361e5fb3-dbce-4f91-86b2-43423a4888d5");
        }
        _ => unreachable!(),
    }
}

#[test]
fn spawn_prepare_progress() {
    let msg = "[23:10:35] [Server thread/INFO]: Preparing spawn area: 44%";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::SpawnPrepareProgress { progress } => {
            assert_eq!(progress, 44);
        }
        _ => unreachable!(),
    }
}

#[test]
fn spawn_prepare_finished() {
    let msg = "[23:10:35] [Server thread/INFO]: Time elapsed: 3292 ms";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::SpawnPrepareFinish { time_elapsed_ms } => {
            assert_eq!(time_elapsed_ms, 3292);
        }
        _ => unreachable!(),
    }
}

#[test]
fn player_lost_connection() {
    let msg = "[19:10:21] [Server thread/INFO]: Cldfire lost connection: Disconnected";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::PlayerLostConnection { name, reason } => {
            assert_eq!(name, "Cldfire");
            assert_eq!(reason, "Disconnected");
        }
        _ => unreachable!(),
    }
}

#[test]
fn player_left_game() {
    let msg = "[19:10:21] [Server thread/INFO]: Cldfire left the game";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::PlayerLogout { name } => {
            assert_eq!(name, "Cldfire");
        }
        _ => unreachable!(),
    }
}

#[test]
fn server_finished_loading() {
    let msg = "[21:57:50] [Server thread/INFO]: Done (7.410s)! For help, type \"help\"";
    let specific_msg =
        ConsoleMsgSpecific::try_parse_from(&ConsoleMsg::try_parse_from(msg).unwrap()).unwrap();

    match specific_msg {
        ConsoleMsgSpecific::FinishedLoading { time_elapsed_s } => {
            assert_eq!(time_elapsed_s, 7.410);
        }
        _ => unreachable!(),
    }
}
