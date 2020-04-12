use chrono::NaiveTime;
use chrono::offset::Local;
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
        coords: (f32, f32, f32),
        /// Present on Spigot servers
        world: Option<String>
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

                let (world, remain) = if remain.starts_with('[') {
                    // This is a Spigot server; parse world
                    let (world, remain) = remain.split_at(remain.find(']').unwrap());
                    (Some(world[1..].to_string()), &remain[1..])
                } else {
                    (None, remain)
                };
                
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
                    coords: (x_coord, y_coord, z_coord),
                    world
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
    /// Create a new `ConsoleMsg` with the current time and a blank thread name.
    pub fn new(msg_type: ConsoleMsgType, msg: String) -> Self {
        Self {
            timestamp: Local::now().naive_local().time(),
            thread_name: "".into(),
            msg_type,
            msg
        }
    }

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
