use chrono::NaiveTime;

#[derive(Debug)]
pub struct ConsoleMsg {
    pub timestamp: NaiveTime,
    pub thread_name: String,
    pub msg_type: ConsoleMsgType,
    pub msg: String
}

impl ConsoleMsg {
    /// Constructs a `ConsoleMsg` from a line of console output.
    pub fn parse_from(raw: &str) -> ConsoleMsg {
        let (mut timestamp, remain) = raw.split_at(raw.find(']').unwrap());
        timestamp = &timestamp[1..];

        let (mut thread_name, remain) = remain.split_at(remain.find('/').unwrap());
        thread_name = &thread_name[3..];

        let (mut msg_type, remain) = remain.split_at(remain.find(']').unwrap());
        msg_type = &msg_type[1..];

        Self {
            timestamp: NaiveTime::from_hms(
                timestamp[..2].parse().unwrap(),
                timestamp[3..5].parse().unwrap(),
                timestamp[6..].parse().unwrap()
            ),
            thread_name: thread_name.into(),
            msg_type: ConsoleMsgType::parse_from(msg_type),
            msg: remain[3..].into()
        }
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
    use super::ConsoleMsg;
    use super::ConsoleMsgType;
    use chrono::Timelike;

    #[test]
    fn parse_warn_msg() {
        let msg = "[23:10:30] [main/WARN]: Ambiguity between arguments [teleport, targets, location] \
            and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]";
        let msg_struct = ConsoleMsg::parse_from(msg);

        assert!(msg_struct.timestamp.hour() == 23);
        assert!(msg_struct.timestamp.minute() == 10);
        assert!(msg_struct.timestamp.second() == 30);
        assert!(msg_struct.thread_name == "main");
        assert!(msg_struct.msg_type == ConsoleMsgType::Warn);
        assert!(msg_struct.msg == "Ambiguity between arguments [teleport, targets, location] \
            and [teleport, targets, destination] with inputs: [0.1 -0.5 .9, 0 0 0]");
    }

    #[test]
    fn parse_info_msg() {
        let msg = "[23:10:31] [Server thread/INFO]: Starting Minecraft server on *:25565";
        let msg_struct = ConsoleMsg::parse_from(msg);

        assert!(msg_struct.timestamp.hour() == 23);
        assert!(msg_struct.timestamp.minute() == 10);
        assert!(msg_struct.timestamp.second() == 31);
        assert!(msg_struct.thread_name == "Server thread");
        assert!(msg_struct.msg_type == ConsoleMsgType::Info);
        assert!(msg_struct.msg == "Starting Minecraft server on *:25565");
    }
}
