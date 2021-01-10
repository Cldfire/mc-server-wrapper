use twilight_mention::parse::MentionType;
use twilight_model::id::{ChannelId, EmojiId, RoleId, UserId};

trait MentionTypeExt: Sized {
    fn try_parse(buf: &str) -> Option<Self>;
}

impl MentionTypeExt for MentionType {
    /// Try to parse the inner part of a mention from the given `buf`.
    ///
    /// This function will parse input such as "@!21984" successfully. It *does
    /// not* handle the < or > characters.
    fn try_parse(buf: &str) -> Option<Self> {
        if let Some(buf) = buf.strip_prefix("@!") {
            // Parse user ID
            buf.parse().ok().map(|n| Self::User(UserId(n)))
        } else if let Some(buf) = buf.strip_prefix("@&") {
            // Parse role ID
            buf.parse().ok().map(|n| Self::Role(RoleId(n)))
        } else if let Some(buf) = buf.strip_prefix('@') {
            // Parse user ID
            buf.parse().ok().map(|n| Self::User(UserId(n)))
        } else if let Some(buf) = buf.strip_prefix(':') {
            // Parse emoji ID (looks like "<:name:123>")
            //
            // Find the second ":"
            buf.find(":")
                // Skip past the second : to get to the ID
                .and_then(|idx| buf.get(idx + 1..))
                .and_then(|s| s.parse().ok())
                .map(|n| Self::Emoji(EmojiId(n)))
        } else if let Some(buf) = buf.strip_prefix('#') {
            // Parse channel ID
            buf.parse().ok().map(|n| Self::Channel(ChannelId(n)))
        } else {
            None
        }
    }
}

trait StrExt {
    fn find_after(&self, idx: usize, slice: &str) -> Option<usize>;
}

impl StrExt for str {
    /// Looks for the given `slice` beginning after the given `idx` in `self`
    fn find_after(&self, idx: usize, slice: &str) -> Option<usize> {
        self.get(idx + 1..)
            .and_then(|s| s.find(slice).map(|next_idx| next_idx + idx + 1))
    }
}

/// Spans parsed out of a Discord message.
#[derive(Debug, Eq, PartialEq)]
pub enum MessageSpan<'a> {
    /// Plain text
    Text(&'a str),
    /// Some sort of mention
    ///
    /// The left side of the tuple is the parsed data and the right side is the
    /// string slice that it was parsed from.
    Mention(MentionType, &'a str),
}

impl<'a> MessageSpan<'a> {
    pub fn iter(buf: &'a str) -> MessageSpanIter<'a> {
        MessageSpanIter { buf, mention: None }
    }
}

#[derive(Debug)]
pub struct MessageSpanIter<'a> {
    buf: &'a str,
    mention: Option<(MentionType, &'a str)>,
}

impl<'a> Iterator for MessageSpanIter<'a> {
    type Item = MessageSpan<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(mention_type) = self.mention.take() {
            // Yield the previously stored mention info if it's present
            Some(MessageSpan::Mention(mention_type.0, mention_type.1))
        } else if let Some((start, end)) = self
            .buf
            .find('<')
            .and_then(|start| self.buf.find_after(start, ">").map(|end| (start, end)))
        {
            // Check and see if we can parse a valid mention
            if let Some(mention_type) = self
                .buf
                .get(start + 1..end)
                .and_then(MentionType::try_parse)
            {
                // Store the mention info to be yielded on the next iteration
                self.mention = Some((mention_type, &self.buf[start..=end]));
                let ret = Some(MessageSpan::Text(&self.buf[..start]));
                self.buf = self.buf.get(end + 1..).unwrap_or("");
                ret
            } else {
                // The mention wasn't valid, yield everything through the > as
                // plain text
                let ret = Some(MessageSpan::Text(&self.buf[..=end]));
                self.buf = self.buf.get(end + 1..).unwrap_or("");
                ret
            }
        } else if !self.buf.is_empty() {
            let ret = Some(MessageSpan::Text(self.buf));
            self.buf = "";
            ret
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use expect_test::{expect, Expect};

    use super::MessageSpan;

    fn check(actual: &str, expect: Expect) {
        let spans = MessageSpan::iter(actual).collect::<Vec<_>>();
        expect.assert_debug_eq(&spans);
    }

    #[test]
    fn empty_string() {
        check(
            "",
            expect![[r#"
            []
        "#]],
        );
    }

    #[test]
    fn various_mention_types() {
        check(
            "channel <#12> emoji <:name:34> role <@&56> user <@78>",
            expect![[r##"
                [
                    Text(
                        "channel ",
                    ),
                    Mention(
                        Channel(
                            ChannelId(
                                12,
                            ),
                        ),
                        "<#12>",
                    ),
                    Text(
                        " emoji ",
                    ),
                    Mention(
                        Emoji(
                            EmojiId(
                                34,
                            ),
                        ),
                        "<:name:34>",
                    ),
                    Text(
                        " role ",
                    ),
                    Mention(
                        Role(
                            RoleId(
                                56,
                            ),
                        ),
                        "<@&56>",
                    ),
                    Text(
                        " user ",
                    ),
                    Mention(
                        User(
                            UserId(
                                78,
                            ),
                        ),
                        "<@78>",
                    ),
                ]
            "##]],
        );
    }
}
