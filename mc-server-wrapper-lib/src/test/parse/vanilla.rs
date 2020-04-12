//! Tests for parsing vanilla console output

use crate::parse::ConsoleMsgSpecific;
use crate::parse::ConsoleMsgType;
use chrono::Timelike;

#[test]
fn warn_msg() {
    let msg = "[23:10:30] [main/WARN]: Ambiguity between arguments [teleport, targets, location] \
        and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::GenericMsg(generic_msg) => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 10);
            assert_eq!(generic_msg.timestamp.second(), 30);
            assert_eq!(generic_msg.thread_name, "main");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Warn);
            assert_eq!(generic_msg.msg, "Ambiguity between arguments [teleport, targets, location] \
                and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn info_msg() {
    let msg = "[23:10:31] [Server thread/INFO]: Starting Minecraft server on *:25565";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::GenericMsg(generic_msg) => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 10);
            assert_eq!(generic_msg.timestamp.second(), 31);
            assert_eq!(generic_msg.thread_name, "Server thread");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "Starting Minecraft server on *:25565");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn must_accept_eula() {
    let msg = "[00:03:56] [Server thread/INFO]: You need to agree to the EULA in order to run the \
        server. Go to eula.txt for more info.";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::MustAcceptEula(generic_msg) => {
            assert_eq!(generic_msg.timestamp.hour(), 00);
            assert_eq!(generic_msg.timestamp.minute(), 03);
            assert_eq!(generic_msg.timestamp.second(), 56);
            assert_eq!(generic_msg.thread_name, "Server thread");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "You need to agree to the EULA in order to run the \
                server. Go to eula.txt for more info.");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_msg() {
    let msg = "[23:12:39] [Server thread/INFO]: <Cldfire> hi!";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerMsg { generic_msg, name, msg } => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 12);
            assert_eq!(generic_msg.timestamp.second(), 39);
            assert_eq!(generic_msg.thread_name, "Server thread");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "<Cldfire> hi!");

            assert_eq!(name, "Cldfire");
            assert_eq!(msg, "hi!");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_login() {
    let msg = "[23:11:12] [Server thread/INFO]: Cldfire[/127.0.0.1:56538] logged in with entity \
        id 121 at (-2.5, 63.0, 256.5)";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerLogin { generic_msg, name, ip, entity_id, coords, world } => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 11);
            assert_eq!(generic_msg.timestamp.second(), 12);
            assert_eq!(generic_msg.thread_name, "Server thread");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "Cldfire[/127.0.0.1:56538] logged in with entity \
                id 121 at (-2.5, 63.0, 256.5)");

            assert_eq!(name, "Cldfire");
            assert_eq!(ip, "127.0.0.1:56538");
            assert_eq!(entity_id, 121);
            assert_eq!(coords, (-2.5, 63.0, 256.5));
            assert!(world.is_none());
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_auth() {
    let msg = "[23:11:12] [User Authenticator #1/INFO]: UUID of player Cldfire is \
        361e5fb3-dbce-4f91-86b2-43423a4888d5";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerAuth { generic_msg, name, uuid } => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 11);
            assert_eq!(generic_msg.timestamp.second(), 12);
            assert_eq!(generic_msg.thread_name, "User Authenticator #1");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "UUID of player Cldfire is \
                361e5fb3-dbce-4f91-86b2-43423a4888d5");

            assert_eq!(name, "Cldfire");
            assert_eq!(uuid, "361e5fb3-dbce-4f91-86b2-43423a4888d5");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn spawn_prepare_progress() {
    let msg = "[23:10:35] [Server thread/INFO]: Preparing spawn area: 44%";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::SpawnPrepareProgress { generic_msg, progress } => {
            assert_eq!(generic_msg.timestamp.hour(), 23);
            assert_eq!(generic_msg.timestamp.minute(), 10);
            assert_eq!(generic_msg.timestamp.second(), 35);
            assert_eq!(generic_msg.thread_name, "Server thread");
            assert_eq!(generic_msg.msg_type, ConsoleMsgType::Info);
            assert_eq!(generic_msg.msg, "Preparing spawn area: 44%");

            assert_eq!(progress, 44);
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn spawn_prepare_finished() {
    let msg = "[23:10:35] [Server thread/INFO]: Time elapsed: 3292 ms";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::SpawnPrepareFinish { generic_msg, time_elapsed_ms } => {
            assert_eq!(generic_msg.msg, "Time elapsed: 3292 ms");
            assert_eq!(time_elapsed_ms, 3292);
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_lost_connection() {
    let msg = "[19:10:21] [Server thread/INFO]: Cldfire lost connection: Disconnected";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerLostConnection { generic_msg, name, reason } => {
            assert_eq!(generic_msg.msg, "Cldfire lost connection: Disconnected");

            assert_eq!(name, "Cldfire");
            assert_eq!(reason, "Disconnected");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn player_left_game() {
    let msg = "[19:10:21] [Server thread/INFO]: Cldfire left the game";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::PlayerLogout { generic_msg, name } => {
            assert_eq!(generic_msg.msg, "Cldfire left the game");

            assert_eq!(name, "Cldfire");
        }
        _ => panic!("wrong variant")
    }
}

#[test]
fn newline() {
    let msg = "\n";
    assert!(ConsoleMsgSpecific::try_parse_from(msg).is_none());
}

#[test]
fn blank_here() {
    // somehow occurs when rapidly firing unknown commands
    let msg = "[19:23:04] [Server thread/INFO]: <--[HERE]";
    let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

    match msg_struct {
        ConsoleMsgSpecific::GenericMsg(_) => {},
        _ => panic!("wrong variant")
    }
}
