# suffix

Rust CLI client for [suffix.org](https://suffix.org), the public API surface for the Suffix shortener.

`suffix` manages owned domains, editable shortcuts, and shortcut statistics through the canonical `https://suffix.org/api/v1` bearer API.

## Login

Seed the CLI with a browser login:

```sh
suffix login person@example.com
```

The CLI opens `suffix.org`, starts a loopback callback on `127.0.0.1`, and waits. Passing an email selects a saved profile for that email, or creates a separate profile named after it. Once a profile has a key, repeating `suffix login EMAIL` reuses that local key and switches to the profile without opening the browser. Use `suffix login EMAIL --renew` only when you intentionally want to replace the key. A new browser approval creates a one-time API key and redirects it to the local listener; the key is then stored in your platform config directory under `suffix/config.toml`. Token persistence and authenticated JSON requests use the public `verdun-cli` base crate; Suffix keeps its domain, shortcut, and transfer routes product-specific.

Suffix depends on the published `verdun-cli = "=0.1.0"` crate, never a sibling checkout. Cargo records that resolved version in `Cargo.lock`, so a Verdun regression cannot alter an existing Suffix build. Update the dependency only as an intentional, separately validated change. Old unnamed `default` profiles are discarded; sign in again with each account's email.

Store several accounts by email or by an explicit profile name:

```sh
suffix login --account personal
suffix login --account work
suffix login personal@example.com
suffix login work@example.com
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
suffix ls [account@example.com]
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
suffix transfer go.example.com --to person@example.com
suffix accept SUF-X7K9-Q2M
suffix mv go.example.com target@example.com
suffix mv --to go.example.com target@example.com
suffix mv --from go.example.com owner@example.com
suffix mv go.example.com owner@example.com target@example.com

suffix account
suffix account ls
suffix account ls -l
suffix account work
suffix account add work --key curtail_sk_...
suffix logout person@example.com
suffix account rm work
suffix man install
```

Bare `suffix` prints the same shortcut view as `suffix ls -l`, then the same cached account view as `suffix account ls -l`.

`suffix ls [EMAIL]` prints tab-separated `shortcut<TAB>target`; `-l`/`--stats` adds visits as the third column. When a saved account email is the trailing argument, Suffix uses that account's key without changing the active profile. `--json`, `--yaml`, and `--xml` emit the same normalized shortcut records in structured form.

`suffix domain ls [EMAIL]` prints tab-separated `domain<TAB>status`; `-l`/`--stats` adds aggregate shortcut visits as the third column. The same optional trailing account email applies to `suffix add`, `suffix rm`, `suffix stats`, `suffix domain add`, and `suffix domain rm`. `--json`, `--yaml`, and `--xml` emit verbose normalized domain records.

`suffix transfer DOMAIN --to EMAIL` creates a short-lived code for moving a domain out of the active account. The receiving account signs in separately and runs `suffix accept CODE`, or pastes the code into the Suffix dashboard. `--to` is optional but recommended because it pins acceptance to that email address.

`suffix mv DOMAIN TARGET_EMAIL_OR_NAME` moves a domain from the active account to the named target account. `--to` is an optional explicit spelling of that direction. `suffix mv --from DOMAIN SOURCE_EMAIL_OR_NAME` moves from the named source account to the active account. `suffix mv DOMAIN FROM_EMAIL_OR_NAME TO_EMAIL_OR_NAME` remains available when you want both accounts explicit. All forms require both saved keys, prompt for `move DOMAIN` before sending the request, and accept `--yes` only for scripts. The source account signs the request, and the target account's saved key proves the receiving account.

`suffix account ls` is offline and prints saved account identities with `logged in` or `logged out`; `-l`/`--long` adds cached base URL, domain/link/visit counts, and any cached status fields. `suffix logout EMAIL_OR_NAME` removes the saved API key but keeps the cached account row.

## Manual page

The crate embeds its maintained `suffix(1)` manual page. Install it after
`cargo install suffix-cli` with:

```sh
suffix man install
man suffix
```

Without `--dir`, Suffix tries standard writable `man1` locations: Homebrew,
`/usr/local`, then user-local on macOS; `/usr/local`, user-local, then
`/usr/share` on Linux. Choose an exact destination when necessary:

```sh
suffix man install --dir /usr/local/share/man/man1
```

## Development

```sh
cargo fmt
cargo test
cargo clippy -- -D warnings
cargo build
```

The repo pins Rust through `rust-toolchain.toml` to the current stable channel with `rustfmt` and `clippy`.

The login flow depends on dashboard support in Suffix: the dashboard only returns API keys to loopback HTTP callbacks with a matching random state.
