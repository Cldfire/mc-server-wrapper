use std::collections::HashSet;

/// Utility function to return a neatly formatted string of online players for
/// a channel topic.
pub fn format_online_players_topic(online_players: &HashSet<String>) -> String {
    // Sort the players for stable name order
    let mut online_players_vec: Vec<_> = online_players.into_iter().collect();
    online_players_vec.sort();

    match online_players.len() {
        0 => "Nobody is playing Minecraft".into(),
        1 => format!("{} is playing Minecraft", online_players_vec[0]),
        2 => format!(
            "{} and {} are playing Minecraft",
            online_players_vec[0], online_players_vec[1]
        ),
        n => {
            let mut string = format!(
                "{}, {}, and {}",
                online_players_vec[0], online_players_vec[1], online_players_vec[2]
            );

            if n > 3 {
                string.push_str(&format!(" (+ {} more)", n - 3));
            }

            string.push_str(" are playing Minecraft");
            string
        }
    }
}

#[cfg(test)]
mod test {
    mod format_online_players_topic {
        use super::super::format_online_players_topic;
        use std::collections::HashSet;

        #[test]
        fn no_players() {
            let online_players = HashSet::new();
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "Nobody is playing Minecraft");
        }

        #[test]
        fn one_player() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1 is playing Minecraft");
        }

        #[test]
        fn two_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1 and p2 are playing Minecraft");
        }

        #[test]
        fn three_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1, p2, and p3 are playing Minecraft");
        }

        #[test]
        fn four_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            online_players.insert("p4".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(
                &formatted,
                "p1, p2, and p3 (+ 1 more) are playing Minecraft"
            );
        }

        #[test]
        fn seven_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            online_players.insert("p4".into());
            online_players.insert("p5".into());
            online_players.insert("p6".into());
            online_players.insert("p7".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(
                &formatted,
                "p1, p2, and p3 (+ 4 more) are playing Minecraft"
            );
        }
    }
}
