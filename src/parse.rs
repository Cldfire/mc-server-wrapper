use chrono::NaiveTime;
use std::fmt;

// TODO: It would be nice to not have the `ConsoleMsg` in every variant
// however, strategies for doing so make it difficult to use `?` in
// `ConsoleMsgSpecific::try_parse_from`...
#[derive(Debug)]
pub enum ConsoleMsgSpecific {
    GenericMsg(ConsoleMsg),
    MustAcceptEula(ConsoleMsg),
    PlayerMsg {
        generic_msg: ConsoleMsg,
        name: String,
        msg: String
    },
    PlayerLogin {
        generic_msg: ConsoleMsg,
        name: String,
        ip: String,
        entity_id: u32,
        coords: (f32, f32, f32)
    },
    PlayerAuth {
        generic_msg: ConsoleMsg,
        name: String,
        uuid: String
    },
    PlayerLogout {
        generic_msg: ConsoleMsg,
        name: String
    },
    PlayerLostConnection {
        generic_msg: ConsoleMsg,
        name: String,
        reason: String
    },
    SpawnPrepareProgress {
        generic_msg: ConsoleMsg,
        progress: u8
    },
    SpawnPrepareFinish {
        generic_msg: ConsoleMsg,
        time_elapsed_ms: u64
    }
}

impl ConsoleMsgSpecific {
    /// Tries to determine a `ConsoleMsgSpecific` variant for a line of console
    /// output.
    pub fn try_parse_from(raw: &str) -> Option<ConsoleMsgSpecific> {
        let parsed = ConsoleMsg::try_parse_from(raw)?;

        // Note that the order in which these conditions are tested is important:
        // we need to make sure that we are not dealing with a player message before
        // it is okay to test for other things, for instance
        Some(if parsed.thread_name.contains("User Authenticator") {
            let (name, uuid) = {
                // Get rid of "UUID of player "
                let minus_start = &parsed.msg[15..];
                let (name, remain) = minus_start.split_at(minus_start.find(' ').unwrap());

                // Slice `remain` to get rid of " is "
                (name.to_string(), remain[4..].to_string())
            };

            ConsoleMsgSpecific::PlayerAuth {
                generic_msg: parsed,
                name,
                uuid
            }
        } else if parsed.msg_type == ConsoleMsgType::Info && (
                parsed.thread_name.starts_with("Async Chat Thread") ||
                parsed.msg.starts_with("<") && parsed.thread_name == "Server thread"
            ) {
                let (name, msg) = {
                    let (name, remain) = parsed.msg.split_at(if let Some(idx) = parsed.msg.find('>') {
                        idx
                    } else {
                        // This is not a player message, return a generic one
                        return Some(ConsoleMsgSpecific::GenericMsg(parsed));
                    });

                    // Trim "<" from the player's name and "> " from the msg
                    (name[1..].to_string(), remain[2..].to_string())
                };

                ConsoleMsgSpecific::PlayerMsg {
                    generic_msg: parsed,
                    name,
                    msg
                }
        } else if parsed.msg == "You need to agree to the EULA in order to run the server. Go to \
                                eula.txt for more info." &&
            parsed.msg_type == ConsoleMsgType::Info {
                ConsoleMsgSpecific::MustAcceptEula(parsed)
        } else if parsed.msg.contains("logged in with entity id") &&
            parsed.msg_type == ConsoleMsgType::Info {
                let (name, remain) = parsed.msg.split_at(parsed.msg.find('[').unwrap());
                let name = name.to_string();

                let (ip, mut remain) = remain.split_at(remain.find(']').unwrap());
                let ip = ip[2..].to_string();

                // Get rid of "] logged in with entity id "
                remain = &remain[27..];

                let (entity_id, mut remain) = remain.split_at(remain.find(' ').unwrap());
                let entity_id = entity_id.parse().unwrap();
                
                // Get rid of " at (" in front and ")" behind
                remain = &remain[5..remain.len() - 1];
                
                // `remain = &remain[2..]` is used to skip ", "
                let (x_coord, mut remain) = remain.split_at(remain.find(',').unwrap());
                remain = &remain[2..];

                let (y_coord, mut remain) = remain.split_at(remain.find(',').unwrap());
                remain = &remain[2..];

                let x_coord = x_coord.parse().unwrap();
                let y_coord = y_coord.parse().unwrap();
                let z_coord = remain.parse().unwrap();


                ConsoleMsgSpecific::PlayerLogin {
                    generic_msg: parsed,
                    name,
                    ip,
                    entity_id,
                    coords: (x_coord, y_coord, z_coord)
                }
        } else if parsed.msg.contains("Preparing spawn area: ") &&
            parsed.msg_type == ConsoleMsgType::Info {
                let progress = parsed.msg[
                    parsed.msg.find(':').unwrap() + 2..parsed.msg.len() - 1
                ].parse().unwrap();

                ConsoleMsgSpecific::SpawnPrepareProgress {
                    generic_msg: parsed,
                    progress
                }
        } else if parsed.msg.contains("Time elapsed: ") {
            let time_elapsed_ms = parsed.msg[
                parsed.msg.find(':').unwrap() + 2..parsed.msg.find("ms").unwrap() - 1
            ].parse().unwrap();

            ConsoleMsgSpecific::SpawnPrepareFinish {
                generic_msg: parsed,
                time_elapsed_ms
            }
        } else if parsed.msg.contains("lost connection: ") {
            let (name, remain) = parsed.msg.split_at(parsed.msg.find(' ').unwrap());
            let name = name.into();
            let reason = remain[remain.find(':').unwrap() + 2..].into();

            ConsoleMsgSpecific::PlayerLostConnection {
                generic_msg: parsed,
                name,
                reason
            }
        } else if parsed.msg.contains("left the game") {
            let name = parsed.msg.split_at(parsed.msg.find(' ').unwrap()).0.into();

            ConsoleMsgSpecific::PlayerLogout {
                generic_msg: parsed,
                name
            }
        } else {
            // It wasn't anything specific we're looking for
            ConsoleMsgSpecific::GenericMsg(parsed)
        })
    }
}

#[derive(Debug)]
pub struct ConsoleMsg {
    pub timestamp: NaiveTime,
    pub thread_name: String,
    pub msg_type: ConsoleMsgType,
    pub msg: String
}

impl fmt::Display for ConsoleMsg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}] [{:?}]: {}",
            self.timestamp.format("%-I:%M:%S %p").to_string(),
            self.msg_type,
            self.msg
        )
    }
}

impl ConsoleMsg {
    /// Constructs a `ConsoleMsg` from a line of console output.
    fn try_parse_from(raw: &str) -> Option<ConsoleMsg> {
        let (mut timestamp, remain) = raw.split_at(raw.find(']')?);
        timestamp = &timestamp[1..];

        let (mut thread_name, remain) = remain.split_at(remain.find('/')?);
        thread_name = &thread_name[3..];

        let (mut msg_type, remain) = remain.split_at(remain.find(']')?);
        msg_type = &msg_type[1..];

        Some(Self {
            timestamp: NaiveTime::from_hms(
                timestamp[..2].parse().unwrap(),
                timestamp[3..5].parse().unwrap(),
                timestamp[6..].parse().unwrap()
            ),
            thread_name: thread_name.into(),
            msg_type: ConsoleMsgType::parse_from(msg_type),
            msg: remain[3..].into()
        })
    }
}

#[derive(Debug, PartialEq)]
pub enum ConsoleMsgType {
    Info,
    Warn,
    Unknown
}

impl ConsoleMsgType {
    fn parse_from(raw: &str) -> Self {
        match raw {
            "INFO" => ConsoleMsgType::Info,
            "WARN" => ConsoleMsgType::Warn,
            _ => ConsoleMsgType::Unknown
        }
    }
}

#[cfg(test)]
mod test {
    use super::ConsoleMsgSpecific;
    use super::ConsoleMsgType;
    use chrono::Timelike;

    #[test]
    fn parse_warn_msg() {
        let msg = "[23:10:30] [main/WARN]: Ambiguity between arguments [teleport, targets, location] \
            and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::GenericMsg(generic_msg) => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 10);
                assert!(generic_msg.timestamp.second() == 30);
                assert!(generic_msg.thread_name == "main");
                assert!(generic_msg.msg_type == ConsoleMsgType::Warn);
                assert!(generic_msg.msg == "Ambiguity between arguments [teleport, targets, location] \
                    and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_info_msg() {
        let msg = "[23:10:31] [Server thread/INFO]: Starting Minecraft server on *:25565";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::GenericMsg(generic_msg) => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 10);
                assert!(generic_msg.timestamp.second() == 31);
                assert!(generic_msg.thread_name == "Server thread");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "Starting Minecraft server on *:25565");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_must_accept_eula() {
        let msg = "[00:03:56] [Server thread/INFO]: You need to agree to the EULA in order to run the \
            server. Go to eula.txt for more info.";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::MustAcceptEula(generic_msg) => {
                assert!(generic_msg.timestamp.hour() == 00);
                assert!(generic_msg.timestamp.minute() == 03);
                assert!(generic_msg.timestamp.second() == 56);
                assert!(generic_msg.thread_name == "Server thread");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "You need to agree to the EULA in order to run the \
                    server. Go to eula.txt for more info.");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_player_msg() {
        let msg = "[23:12:39] [Server thread/INFO]: <Cldfire> hi!";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::PlayerMsg { generic_msg, name, msg } => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 12);
                assert!(generic_msg.timestamp.second() == 39);
                assert!(generic_msg.thread_name == "Server thread");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "<Cldfire> hi!");

                assert!(name == "Cldfire");
                assert!(msg == "hi!");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_player_msg_spigot() {
        let msg = "[23:12:39] [Async Chat Thread - #8/INFO]: <Cldfire> hi!";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::PlayerMsg { generic_msg, name, msg } => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 12);
                assert!(generic_msg.timestamp.second() == 39);
                assert!(generic_msg.thread_name == "Async Chat Thread - #8");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "<Cldfire> hi!");

                assert!(name == "Cldfire");
                assert!(msg == "hi!");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_player_login() {
        let msg = "[23:11:12] [Server thread/INFO]: Cldfire[/127.0.0.1:56538] logged in with entity \
            id 121 at (-2.5, 63.0, 256.5)";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::PlayerLogin { generic_msg, name, ip, entity_id, coords } => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 11);
                assert!(generic_msg.timestamp.second() == 12);
                assert!(generic_msg.thread_name == "Server thread");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "Cldfire[/127.0.0.1:56538] logged in with entity \
                    id 121 at (-2.5, 63.0, 256.5)");

                assert!(name == "Cldfire");
                assert!(ip == "127.0.0.1:56538");
                assert!(entity_id == 121);
                assert!(coords == (-2.5, 63.0, 256.5));
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_player_auth() {
        let msg = "[23:11:12] [User Authenticator #1/INFO]: UUID of player Cldfire is \
            361e5fb3-dbce-4f91-86b2-43423a4888d5";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::PlayerAuth { generic_msg, name, uuid } => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 11);
                assert!(generic_msg.timestamp.second() == 12);
                assert!(generic_msg.thread_name == "User Authenticator #1");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "UUID of player Cldfire is \
                    361e5fb3-dbce-4f91-86b2-43423a4888d5");

                assert!(name == "Cldfire");
                assert!(uuid == "361e5fb3-dbce-4f91-86b2-43423a4888d5");
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_spawn_prepare_progress() {
        let msg = "[23:10:35] [Server thread/INFO]: Preparing spawn area: 44%";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::SpawnPrepareProgress { generic_msg, progress } => {
                assert!(generic_msg.timestamp.hour() == 23);
                assert!(generic_msg.timestamp.minute() == 10);
                assert!(generic_msg.timestamp.second() == 35);
                assert!(generic_msg.thread_name == "Server thread");
                assert!(generic_msg.msg_type == ConsoleMsgType::Info);
                assert!(generic_msg.msg == "Preparing spawn area: 44%");

                assert!(progress == 44);
            }
            _ => panic!("wrong variant")
        }
    }

    #[test]
    fn parse_spawn_prepare_finished() {
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
    fn parse_player_lost_connection() {
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
    fn parse_player_left_game() {
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
    fn parse_newline() {
        let msg = "\n";
        assert!(ConsoleMsgSpecific::try_parse_from(msg).is_none());
    }

    #[test]
    fn parse_loading_libraries() {
        // spigot prints this non-standard line without a timestamp
        let msg = "Loading libraries, please wait...";
        assert!(ConsoleMsgSpecific::try_parse_from(msg).is_none());
    }

    #[test]
    fn parse_blank_here() {
        // somehow occurs when rapidly firing unknown commands
        let msg = "[19:23:04] [Server thread/INFO]: <--[HERE]";
        let msg_struct = ConsoleMsgSpecific::try_parse_from(msg).unwrap();

        match msg_struct {
            ConsoleMsgSpecific::GenericMsg(_) => {},
            _ => panic!("wrong variant")
        }
    }
}
