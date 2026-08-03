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
suffix search 'foto\\.gs|report.*pdf'
suffix add pair.rs/typesec https://github.com/querygraph/typesec
suffix add -d pair.rs typesec https://github.com/querygraph/typesec
suffix add https://example.com launch --domain-id <uuid>
suffix add foto.gs/sunset https://example.com/sunset.jpg --photo --photo-mode local
suffix photo <shortcut-id> -r
suffix photo <shortcut-id> --drop
suffix edit <shortcut-id> --tail new-tail --target-url https://example.com/new
suffix password <shortcut-id> --add
suffix password <shortcut-id> --remove
suffix upload files.example/report ./report.pdf
suffix upload files.example/private ./report.pdf --protect
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

Top-tier accounts can create a Photo shortcut with `suffix add ... --photo`. `--photo-mode local` asks Suffix to keep and directly serve a managed copy of the target image; `remote` keeps normal resolution to the original and is the default. Change that default later with `suffix photo SHORTCUT_ID -l`/`--local` or `-r`/`--remote`, or remove the managed photo with `--drop` (and optionally `--version VERSION`). On the public Photo URL, `?L` requests the local copy and `?R` requests the remote original for that request without changing the saved default. Plan enforcement, image validation/ingestion, storage, and these request overrides are implemented by the Suffix API and Vercel application; the CLI sends the corresponding fields.

Business accounts can upload a local file and create its short URL in one step with `suffix upload HOST/TAIL PATH`. Files go directly to an account-scoped Vercel Blob path and may be up to 100 MB. The short URL downloads the managed file, and deleting the shortcut removes its Blob object. `-d HOST TAIL PATH`, `--domain-id`, `--title`, and a trailing saved-account email follow the same conventions as `suffix add`.

Use `--protect` on a stored photo or upload to prompt without echoing for an
8–128 character password. Automation can use `--password-file PATH`; passwords
are never accepted as command-line values or printed. Photo protection applies
to local mode and `?L`, while `?R` remains the remote target.
Use `suffix password SHORTCUT_ID --add` to add or change protection on an
existing managed file or stored photo, and `--remove` to remove it.

`suffix edit SHORTCUT_ID` changes any combination of `--tail`, `--target-url`,
`--title`, and `--active`. `suffix rm SHORTCUT_ID` deletes the shortcut and its
managed object. Both commands discover the current version unless one is supplied.

`suffix search REGEXP [EMAIL]` searches both `hostname/tail` and destination URL. Its first tab-separated column reports `shortcut`, `target`, or `shortcut+target` to show why each row matched.

Before `suffix add` creates a shortcut, it checks the current inventory. An occupied tail offers to edit the existing shortcut; an existing target offers to reuse its old shortcut or confirm creating another. Interactive terminals prompt for the choice. Automation uses `--edit-existing` or `--allow-duplicate-target` explicitly and otherwise fails closed.

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
