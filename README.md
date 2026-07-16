# suffix

Rust CLI client for [suffix.org](https://suffix.org), the public API surface for the Curtail shortener.

`suffix` manages owned domains, editable shortcuts, and shortcut statistics through the canonical `https://suffix.org/api/v1` bearer API.

## Login

Seed the CLI with a browser login:

```sh
suffix login
```

The CLI opens `suffix.org`, starts a loopback callback on `127.0.0.1`, and waits. After normal browser sign-in, the dashboard asks you to approve a one-time API key for the CLI and redirects the secret back to the local listener. The key is stored in your platform config directory under `suffix/config.toml`.

Store several accounts by naming each login:

```sh
suffix login --account personal
suffix login --account work
suffix account
suffix account personal
```

Use `--no-open` to print the login URL instead of launching a browser:

```sh
suffix login --account work --no-open
```

Environment variables override saved config:

```sh
export SUFFIX_API_BASE=https://suffix.org/api/v1
export SUFFIX_API_KEY=curtail_sk_...
```

## Commands

```sh
suffix ls
suffix ls --stats
suffix ls --json
suffix ls --yaml
suffix ls --xml
suffix add https://example.com launch --domain-id <uuid>
suffix rm <shortcut-id> --version <version>
suffix stats <shortcut-id> --days 30

suffix domain ls
suffix domain add go.example.com
suffix domain rm <domain-id>

suffix account
suffix account work
suffix account add work --key curtail_sk_...
suffix account rm work
```

`suffix ls` prints tab-separated `shortcut<TAB>target`; `--stats` adds visits as the third column. `--json`, `--yaml`, and `--xml` emit the same normalized shortcut records in structured form.

## Development

```sh
cargo fmt
cargo test
cargo clippy -- -D warnings
cargo build
```

The repo pins Rust through `rust-toolchain.toml` to the current stable channel with `rustfmt` and `clippy`.

The login flow depends on dashboard support in Curtail: the dashboard only returns API keys to loopback HTTP callbacks with a matching random state.
