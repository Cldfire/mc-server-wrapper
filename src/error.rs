/// General application errors
#[derive(Debug)]
pub enum Error {
    DiscordErr(twilight::http::Error),
}

impl From<twilight::http::Error> for Error {
    fn from(err: twilight::http::Error) -> Self {
        Self::DiscordErr(err)
    }
}
