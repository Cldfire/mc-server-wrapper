# How to release mc-server-wrapper

1. Make the necessary updates to [CHANGELOG.md](./CHANGELOG.md).
2. Bump version in [Cargo.toml](./mc-server-wrapper/Cargo.toml).
3. Commit changes.
4. Tag new version and push tag.
    ```
    git tag -sm "<version>" <version>
    git push origin <version>
    ```
5. Confirm the release is built and published appropriately by CI.
