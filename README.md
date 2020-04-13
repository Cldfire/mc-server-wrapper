# mc-server-wrapper

Lightweight Rust program to run a Java Minecraft server (vanilla, Spigot, or PaperSpigot), providing niceties such as a Discord chat bridge, server restart-on-crash, and improved console output.

Not production-ready, but getting there. Feel free to use it for non-critical servers, and please report issues!

## Features

* Optionally enabled bi-directional Discord chat bridge (see [Discord Bridge Setup](#discord-bridge-setup))
* Run server with configurable memory allocation
* Restart server on crash
* Auto-agree to EULA
* Improved console output formatting

## Discord bridge setup

Register an application and a bot with [Discord](https://discordapp.com/developers/applications). Add the bot to the guild you want to bridge to and get the ID of the channel you want to bridge to (Google this for instructions). Provide the bot token and channel ID either through the CLI or through the evironment variables listed below:

```
DISCORD_TOKEN="..."
DISCORD_CHANNEL_ID="..."
```

These environment variables can also be provided in a `.env` file of the above format. See [dotenv](https://github.com/dotenv-rs/dotenv) for more.

## Future plans

* Simple web and CLI interface to administrate server
    * Change most common settings
    * View online players
    * Chat from the web
    * Different levels of accounts (user, admin)
* _further ideas here_

## Library

The binary for this project is built on top of a library; if you want to implement a different feature set than the one I've chosen to, or implement the features in a different way, you can easily do so. See the [`mc-server-wrapper-lib` README](mc-server-wrapper-lib/README.md) for more.
