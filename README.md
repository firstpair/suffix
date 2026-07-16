# suffix

Rust CLI client for [suffix.org](https://suffix.org), the public API surface for the Suffix shortener.

`suffix` manages owned domains, editable shortcuts, and shortcut statistics through the canonical `https://suffix.org/api/v1` bearer API.

## Login

Seed the CLI with a browser login:

```sh
suffix login
suffix login person@example.com
```

The CLI opens `suffix.org`, starts a loopback callback on `127.0.0.1`, and waits. Passing an email sends it as a browser sign-in hint, which is useful when you need to mint a key for another Suffix account. After normal browser sign-in, the dashboard asks you to approve a one-time API key for the CLI and redirects the secret back to the local listener. The key is stored in your platform config directory under `suffix/config.toml`.

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
suffix
suffix ls
suffix ls -l
suffix ls --json
suffix ls --yaml
suffix ls --xml
suffix add pair.rs/typesec https://github.com/querygraph/typesec
suffix add -d pair.rs typesec https://github.com/querygraph/typesec
suffix add https://example.com launch --domain-id <uuid>
suffix rm <shortcut-id> --version <version>
suffix stats <shortcut-id> --days 30

suffix domain ls
suffix domain ls -l
suffix domain ls --json
suffix domain ls --yaml
suffix domain ls --xml
suffix domain add go.example.com
suffix domain rm <domain-id>
suffix mv go.example.com owner@example.com target@example.com

suffix account
suffix account ls
suffix account ls -l
suffix account work
suffix account add work --key curtail_sk_...
suffix logout person@example.com
suffix account rm work
```

Bare `suffix` prints the same shortcut view as `suffix ls -l`, then the same cached account view as `suffix account ls -l`.

`suffix ls` prints tab-separated `shortcut<TAB>target`; `-l`/`--stats` adds visits as the third column. `--json`, `--yaml`, and `--xml` emit the same normalized shortcut records in structured form.

`suffix domain ls` prints tab-separated `domain<TAB>status`; `-l`/`--stats` adds aggregate shortcut visits as the third column. `--json`, `--yaml`, and `--xml` emit verbose normalized domain records.

`suffix mv DOMAIN FROM_EMAIL_OR_NAME TO_EMAIL_OR_NAME` transfers a domain between two locally logged-in accounts. It prompts for `move DOMAIN` before sending the request; use `--yes` only for scripts. The source account signs the request, and the target account's saved key proves the receiving account.

`suffix account ls` is offline and prints saved account identities with `logged in` or `logged out`; `-l`/`--long` adds cached base URL, domain/link/visit counts, and any cached status fields. `suffix logout EMAIL_OR_NAME` removes the saved API key but keeps the cached account row.

## Development

```sh
cargo fmt
cargo test
cargo clippy -- -D warnings
cargo build
```

The repo pins Rust through `rust-toolchain.toml` to the current stable channel with `rustfmt` and `clippy`.

The login flow depends on dashboard support in Suffix: the dashboard only returns API keys to loopback HTTP callbacks with a matching random state.
