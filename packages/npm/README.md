# OwnSERP

An MCP server for the Serper.dev search API. This package downloads the pinned Linux x86_64 release binary on first run, checks it against the published SHA-256, and starts it with the arguments you pass.

```sh
npx @devops-infinity/ownserp serve
```

The server reads `SERPER_API_KEY` from the environment. For macOS, Windows, or another architecture, install with `cargo install ownserp` or the Homebrew tap at `devops-infinity/tap`.

The repository is at https://github.com/devops-infinity/ownserp.
