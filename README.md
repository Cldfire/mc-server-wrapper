# mc-server-wrapper

![CI](https://github.com/Cldfire/mc-server-wrapper/workflows/CI/badge.svg)

Lightweight Rust program to manage a Java Minecraft server process (vanilla, Spigot, or PaperSpigot), providing niceties such as a Discord chat bridge, server restart-on-crash, and improved console output.

Not production-ready, but getting there. Feel free to use it for non-critical servers, and please report issues!

## Features

* Optionally enabled bi-directional Discord chat bridge (see [Discord Bridge Setup](#discord-bridge-setup))
    * Commands (prefixed by `!mc`):
        * `list`: replies with a list of people playing Minecraft
    * Server topic displays list of online players
    * Embeds, mentions, and attachments in Discord messages are neatly formatted in Minecraft
* Run server with configurable memory allocation
    * Also allows passing custom JVM flags if desired
* Restart server on crash
* Auto-agree to EULA
* Improved console output formatting

## Discord bridge setup

* Register an application and a bot with [Discord](https://discordapp.com/developers/applications)
* Toggle `Server Members Intent` to on under `Privileged Gateway Intents`
  * This is used to receive member updates from your guild (such as when someone changes their nickname so we can change the name we display in-game)
* Add the bot to the guild you want to bridge to
* Get the ID of the channel you want to bridge to (Google this for instructions)
* Provide the bot token and channel ID either through the CLI or through the evironment variables listed below:

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

The binary for this project is built on top of a library; if you want to implement a different feature set than the one I've chosen to, or implement the features in a different way, you can easily do so. See the [`mc-server-wrapper-lib` README](mc-server-wrapper-lib/README.md) and its [basic example](mc-server-wrapper-lib/examples/basic.rs) for more.

```
cargo run --example basic -- path/to/server.jar
```

## Screenshot

Early screenshot, subject to change:

![demo screenshot showing off the TUI](tui-demo.png)

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
