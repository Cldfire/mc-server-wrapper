use std::collections::HashSet;

/// Utility function to return a neatly formatted string describing who's playing
/// Minecraft
///
/// `short` can be set to true to truncate the list.
pub fn format_online_players(online_players: &HashSet<String>, short: bool) -> String {
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
            if short {
                let mut string = format!(
                    "{}, {}, and {}",
                    online_players_vec[0], online_players_vec[1], online_players_vec[2]
                );

                if n > 3 {
                    string.push_str(&format!(" (+ {} more)", n - 3));
                }

                string.push_str(" are playing Minecraft");
                string
            } else {
                let mut string = String::new();

                for player in online_players_vec[..online_players_vec.len() - 1].iter() {
                    string.push_str(&format!("{}, ", player));
                }

                string.push_str(&format!(
                    "and {} are playing Minecraft",
                    online_players_vec.last().unwrap()
                ));
                string
            }
        }
    }
}

#[cfg(test)]
mod test {
    mod format_online_players {
        use super::super::format_online_players;
        use std::collections::HashSet;

        mod common {
            use super::*;

            #[test]
            fn no_players() {
                let online_players = HashSet::new();
                let expected = "Nobody is playing Minecraft";

                let formatted = format_online_players(&online_players, true);
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(&online_players, false);
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn one_player() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                let expected = "p1 is playing Minecraft";

                let formatted = format_online_players(&online_players, true);
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(&online_players, false);
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn two_players() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
                let expected = "p1 and p2 are playing Minecraft";

                let formatted = format_online_players(&online_players, true);
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(&online_players, false);
                assert_eq!(&formatted, expected);
            }

            #[test]
            fn three_players() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
                online_players.insert("p3".into());
                let expected = "p1, p2, and p3 are playing Minecraft";

                let formatted = format_online_players(&online_players, true);
                assert_eq!(&formatted, expected);

                let formatted = format_online_players(&online_players, false);
                assert_eq!(&formatted, expected);
            }
        }

        mod short {
            use super::*;

            #[test]
            fn four_players() {
                let mut online_players = HashSet::new();
                online_players.insert("p1".into());
                online_players.insert("p2".into());
                online_players.insert("p3".into());
                online_players.insert("p4".into());
                let formatted = format_online_players(&online_players, true);

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
                let formatted = format_online_players(&online_players, true);

                assert_eq!(
                    &formatted,
                    "p1, p2, and p3 (+ 4 more) are playing Minecraft"
                );
            }
        }

        mod long {
            use super::*;

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
                let formatted = format_online_players(&online_players, false);

                assert_eq!(
                    &formatted,
                    "p1, p2, p3, p4, p5, p6, and p7 are playing Minecraft"
                );
            }
        }
    }
}
