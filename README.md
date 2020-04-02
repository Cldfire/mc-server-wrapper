# mc-wrapper

WIP Rust program to run a Java Minecraft server (vanilla, Spigot, or PaperSpigot), providing niceties such as restart-on-crash and improved console output.

## Discord bridge setup

Register an application and a bot with [Discord](https://discordapp.com/developers/applications). Provide the bot token either through the CLI, through the evironment variable `DISCORD_TOKEN`, or inside of a `.env` file containing the following:

```
DISCORD_TOKEN="..."
```

See [dotenv](https://github.com/dotenv-rs/dotenv) for more.

## Current status

* Run server with configurable memory allocation
* Restart server on crash
* Auto-agree to EULA
* Improved console output formatting

## Future plans

* Simple web and CLI interface to administrate server
    * Change most common settings
    * View online players
    * Chat from the web
    * Different levels of accounts (user, admin)
* Discord bridge?
* _further ideas here_
