use chrono::offset::Local;
use chrono::NaiveTime;
use std::fmt;

/// More informative representations for specific, supported console messages.
#[derive(Debug, PartialEq)]
pub enum ConsoleMsgSpecific {
    MustAcceptEula,
    PlayerMsg {
        name: String,
        msg: String,
    },
    PlayerLogin {
        name: String,
        ip: String,
        entity_id: u32,
        coords: (f32, f32, f32),
        /// Present on Spigot servers
        world: Option<String>,
    },
    PlayerAuth {
        name: String,
        uuid: String,
    },
    PlayerLogout {
        name: String,
    },
    PlayerLostConnection {
        name: String,
        reason: String,
    },
    SpawnPrepareProgress {
        progress: u8,
    },
    SpawnPrepareFinish {
        time_elapsed_ms: u64,
    },
}

impl ConsoleMsgSpecific {
    /// Tries to determine a `ConsoleMsgSpecific` variant for the given
    /// `ConsoleMsg`.
    pub(crate) fn try_parse_from(console_msg: &ConsoleMsg) -> Option<ConsoleMsgSpecific> {
        // Note that the order in which these conditions are tested is important:
        // we need to make sure that we are not dealing with a player message before
        // it is okay to test for other things, for instance
        Some(if console_msg.thread_name.contains("User Authenticator") {
            let (name, uuid) = {
                // Get rid of "UUID of player "
                let minus_start = &console_msg.msg[15..];
                let (name, remain) = minus_start.split_at(minus_start.find(' ').unwrap());

                // Slice `remain` to get rid of " is "
                (name.to_string(), remain[4..].to_string())
            };

            ConsoleMsgSpecific::PlayerAuth { name, uuid }
        } else if console_msg.msg_type == ConsoleMsgType::Info
            && (console_msg.thread_name.starts_with("Async Chat Thread")
                || console_msg.msg.starts_with('<') && console_msg.thread_name == "Server thread")
        {
            let (name, msg) = {
                let (name, remain) = console_msg
                    .msg
                    // If a > cannot be found, this is not a player message
                    // and therefore we return
                    .split_at(console_msg.msg.find('>')?);

                // Trim "<" from the player's name and "> " from the msg
                (name[1..].to_string(), remain[2..].to_string())
            };

            ConsoleMsgSpecific::PlayerMsg { name, msg }
        } else if console_msg.msg
            == "You need to agree to the EULA in order to run the server. Go to \
                                eula.txt for more info."
            && console_msg.msg_type == ConsoleMsgType::Info
        {
            ConsoleMsgSpecific::MustAcceptEula
        } else if console_msg.msg.contains("logged in with entity id")
            && console_msg.msg_type == ConsoleMsgType::Info
        {
            let (name, remain) = console_msg.msg.split_at(console_msg.msg.find('[').unwrap());
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
                name,
                ip,
                entity_id,
                coords: (x_coord, y_coord, z_coord),
                world,
            }
        } else if console_msg.msg.contains("Preparing spawn area: ")
            && console_msg.msg_type == ConsoleMsgType::Info
        {
            let progress = console_msg.msg
                [console_msg.msg.find(':').unwrap() + 2..console_msg.msg.len() - 1]
                .parse()
                .unwrap();

            ConsoleMsgSpecific::SpawnPrepareProgress { progress }
        } else if console_msg.msg.contains("Time elapsed: ") {
            let time_elapsed_ms = console_msg.msg
                [console_msg.msg.find(':').unwrap() + 2..console_msg.msg.find("ms").unwrap() - 1]
                .parse()
                .unwrap();

            ConsoleMsgSpecific::SpawnPrepareFinish { time_elapsed_ms }
        } else if console_msg.msg.contains("lost connection: ") {
            let (name, remain) = console_msg.msg.split_at(console_msg.msg.find(' ').unwrap());
            let name = name.into();
            let reason = remain[remain.find(':').unwrap() + 2..].into();

            ConsoleMsgSpecific::PlayerLostConnection { name, reason }
        } else if console_msg.msg.contains("left the game") {
            let name = console_msg
                .msg
                .split_at(console_msg.msg.find(' ').unwrap())
                .0
                .into();

            ConsoleMsgSpecific::PlayerLogout { name }
        } else {
            // It wasn't anything specific we're looking for
            return None;
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct ConsoleMsg {
    pub timestamp: NaiveTime,
    pub thread_name: String,
    pub msg_type: ConsoleMsgType,
    pub msg: String,
}

impl fmt::Display for ConsoleMsg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "[{}] [mc, {:?}]: {}",
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
            msg,
        }
    }

    /// Constructs a `ConsoleMsg` from a line of console output.
    pub(crate) fn try_parse_from(raw: &str) -> Option<ConsoleMsg> {
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
                timestamp[6..].parse().unwrap(),
            ),
            thread_name: thread_name.into(),
            msg_type: ConsoleMsgType::parse_from(msg_type),
            msg: remain[3..].into(),
        })
    }
}

/// Various types of console messages that can occur
#[derive(Debug, PartialEq)]
pub enum ConsoleMsgType {
    Info,
    Warn,
    Error,
    /// An unknown type of message
    Unknown,
}

impl ConsoleMsgType {
    fn parse_from(raw: &str) -> Self {
        match raw {
            "INFO" => ConsoleMsgType::Info,
            "WARN" => ConsoleMsgType::Warn,
            "ERROR" => ConsoleMsgType::Error,
            _ => ConsoleMsgType::Unknown,
        }
    }
}
