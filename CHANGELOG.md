# mc-server-wrapper changelog

Notable `mc-server-wrapper` changes, tracked in the [keep a changelog](https://keepachangelog.com/en/1.0.0/) format with the addition of the `Internal` change type.

## [Unreleased]

### Changed

* We no longer attempt to fetch missing member info via Discord's HTTP API if it is not present in the cache ([explanation](https://github.com/twilight-rs/twilight/pull/437))
  * This should not have any user-facing impact because technically speaking all needed info will be present in the cache anyway

### Fixed

* Minecraft player names are no longer markdown-sanitized for display in the bot status (the bot status message doesn't support markdown)

### Internal

* Updated to `twilight` 0.1 release from crates.io
* Replaced custom mention parsing code with the new `twilight-mention` support for parsing mentions from Discord messages

## [alpha2] - 2020-08-22

### Added

* The bot's status message is now updated with server information (such as the names of online players)
* Content that failed to validate will now be logged alongside the warning that a message failed to send to Discord

## [alpha1] - 2020-07-26

Tagging a first alpha release after a few months of working on the project.

[Unreleased]: https://github.com/Cldfire/mc-server-wrapper/compare/alpha2...HEAD
[alpha2]: https://github.com/Cldfire/mc-server-wrapper/compare/alpha1...alpha2
[alpha1]: https://github.com/Cldfire/mc-server-wrapper/releases/tag/alpha1
