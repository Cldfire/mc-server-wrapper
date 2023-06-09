# How to release mc-server-wrapper

1. Make the necessary updates to [CHANGELOG.md](./CHANGELOG.md).
2. Bump version in [Cargo.toml](./Cargo.toml).
3. Tag new version and push tag.
    ```
    git tag -sm "<version>" <version>
    git push origin <version>
    ```
4. Confirm the release is built and published appropriately by CI.
