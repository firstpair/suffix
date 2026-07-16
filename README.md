# suffix

Rust CLI client for [suffix.org](https://suffix.org), the public API surface for the Curtail shortener.

`suffix` manages owned domains, editable shortcuts, and shortcut statistics through the canonical `https://suffix.org/api/v1` bearer API.

## Login

Seed the CLI with a browser login:

```sh
cargo run -- login
```

The CLI opens `suffix.org`, starts a loopback callback on `127.0.0.1`, and waits. After normal browser sign-in, the dashboard asks you to approve a one-time API key for the CLI and redirects the secret back to the local listener. The key is stored in your platform config directory under `suffix/config.toml`.

Use `--no-open` to print the login URL instead of launching a browser:

```sh
cargo run -- login --no-open
```

Environment variables override saved config:

```sh
export SUFFIX_API_KEY=curtail_sk_...
export SUFFIX_API_BASE=https://suffix.org/api/v1
```

## Commands

```sh
suffix domains list
suffix domains add --hostname go.example.com
suffix shortcuts list
suffix shortcuts create --domain-id <uuid> --tail launch --target-url https://example.com
suffix stats <shortcut-id> --days 30
```

All commands print formatted JSON from the API.

## Development

```sh
cargo fmt
cargo test
cargo build
```

The login flow depends on dashboard support in Curtail: the dashboard only returns API keys to loopback HTTP callbacks with a matching random state.
