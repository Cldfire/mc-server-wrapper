/// General application errors
#[derive(Debug)]
pub enum Error {
    DiscordErr(twilight::http::Error),
    DiscordClusterErr(twilight::gateway::cluster::Error),
}

impl From<twilight::http::Error> for Error {
    fn from(err: twilight::http::Error) -> Self {
        Self::DiscordErr(err)
    }
}

impl From<twilight::gateway::cluster::Error> for Error {
    fn from(err: twilight::gateway::cluster::Error) -> Self {
        Self::DiscordClusterErr(err)
    }
}
