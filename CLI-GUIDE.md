---
title: SUFFIX
section: 1
header: User Commands
footer: Suffix 0.1.5
date: 2026-08-08
---

# NAME

suffix - manage durable short links, domains, photos, and files on suffix.org

# SYNOPSIS

`suffix [COMMAND] [ARGUMENTS] [OPTIONS]`

`suffix man [show]`

`suffix man install [-d|--dir DIR]`

# DESCRIPTION

Suffix is the command-line client for `https://suffix.org/api/v1`. It manages shortcuts and domains belonging to a saved account, uses Somme for configuration and authenticated requests, and supports multiple local account profiles. With no command it prints an overview of the active account's shortcuts and saved profiles.

All optional arguments have equivalent short and long forms. Positional account selectors operate for one invocation without changing the active profile.

# COMMANDS

## login

`suffix login [EMAIL] [-u|--app-url URL] [-e|--email EMAIL] [-a|--account NAME] [-n|--name KEY-NAME] [-r|--renew] [-o|--no-open] [-t|--timeout SECONDS]`

Select a matching saved account or start browser approval and store a new API key. The positional email and `--email` are equivalent and conflict when both are supplied. A matching stored key is reused unless `--renew` is present. `--account` selects the local profile name, `--name` labels a newly minted dashboard key, `--app-url` changes the approval site, `--no-open` prints rather than opens the URL, and `--timeout` defaults to 120 seconds.

## logout

`suffix logout ACCOUNT`

Forget the selected profile's API key while retaining cached metadata.

## ls

`suffix ls [ACCOUNT] [-j|--json] [-x|--xml] [-y|--yaml] [-l|--stats]`

List shortcuts as tab-separated shortcut and target columns. `--stats` adds visit counts. Exactly one structured output flag may be selected. The hidden compatibility flag `-d|--domains` lists domains; prefer `suffix domain ls`.

## search

`suffix search REGEXP [ACCOUNT]`

Search shortcut URLs and targets using a case-insensitive regular expression. Output is tab-separated match scope, shortcut, and target.

## add

`suffix add [OPTIONS] VALUE [TAIL] [TARGET-URL] [ACCOUNT]`

Create a redirect or Photo shortcut. `VALUE` may be `HOST/TAIL`, a target URL, or a public tail depending on the selected form.

Options:

`-d, --domain HOST`
: Explicit shortcut hostname.

`-p, --public`
: Use public `suf.cx`; omit a tail for generated candidates or provide a custom tail before the URL.

`-l, --letters`; `-a, --alphanumeric`; `-w, --words`
: Candidate styles for `--public`.

`-i, --domain-id ID`
: Domain ID; defaults to the first owned domain.

`-t, --title TITLE`
: Optional shortcut label.

`-P, --photo`
: Create a Photo shortcut.

`-m, --photo-mode local|remote`
: Stored or original image mode; requires `--photo` and defaults to remote.

`-D, --allow-duplicate-target`
: Create another shortcut even when the target already has one.

`-e, --edit-existing`
: Replace an occupied tail rather than failing.

`-r, --protect`
: Password-protect the stored photo; requires `--photo`.

`-f, --password-file PATH`
: Read the storage password from a file instead of prompting; requires `--photo`.

## edit

`suffix edit SHORTCUT-ID [-t|--tail TAIL] [-u|--target-url URL] [-n|--title TITLE] [-a|--active BOOL] [-v|--version VERSION] [ACCOUNT]`

Edit selected shortcut fields. An empty title removes it. If `--version` is omitted, Suffix reads the current version before performing the optimistic update.

## upload

`suffix upload [-d|--domain HOST] VALUE FILE [ACCOUNT] [-i|--domain-id ID] [-t|--title TITLE] [-p|--protect] [-f|--password-file PATH]`

Upload a Business file of up to 100 MB and create a shortcut that downloads it. `VALUE` is `HOST/TAIL`, or just `TAIL` with `--domain`. Deleting the shortcut removes its managed object.

## rm

`suffix rm SHORTCUT-ID [ACCOUNT] [-v|--version VERSION]`

Delete a shortcut, looking up its version when omitted. The hidden `-d|--domain` compatibility flag removes a domain; prefer `suffix domain rm`.

## photo

`suffix photo SHORTCUT-ID (-l|--local|-r|--remote|-d|--drop) [ACCOUNT] [-v|--version VERSION]`

Select stored-image mode, original-image mode, or remove the managed image. Public Photo URLs accept `?L` and `?R` one-request overrides. The three mode flags conflict.

## password

`suffix password SHORTCUT-ID (-a|--add|-r|--remove) [ACCOUNT] [-f|--password-file PATH] [-v|--version VERSION]`

Add, change, or remove password protection on a managed photo or file. Without `--password-file`, adding prompts without echo.

## mv

`suffix mv DOMAIN [ACCOUNT] [TARGET] [-t|--to|-f|--from] [-y|--yes]`

Move a domain between saved accounts. The default and `--to` direction move from the active account to `ACCOUNT`; `--from` reverses it. Supplying `TARGET` explicitly identifies both source and destination. `--yes` skips confirmation.

## transfer

`suffix transfer DOMAIN [-t|--to EMAIL] [-m|--minutes MINUTES]`

Create a short-lived domain transfer code. `--to` restricts acceptance to one email. Expiration defaults to 15 minutes.

## accept

`suffix accept CODE`

Accept a domain transfer code into the active account.

## domain

`suffix domain ls [ACCOUNT] [-j|--json] [-x|--xml] [-y|--yaml] [-l|--stats]`

List domains. Structured formats are mutually exclusive; `--stats` includes aggregate visits.

`suffix domain add HOSTNAME [ACCOUNT]`

Add a domain to the selected account.

`suffix domain rm ID [ACCOUNT]`

Remove an empty domain.

## account

`suffix account [-l|--long] [-e|--email EMAIL] [-k|--key KEY] [-u|--api-base URL] [ARGS...]`

With no arguments or `ls`, list profiles. A single profile name selects it. `add NAME --key KEY` stores a profile, with optional email and API base. `rm NAME` forgets it. `--long` includes cached API base, inventory counts, refresh state, and errors. The default API base is `https://suffix.org/api/v1`.

## stats

`suffix stats SHORTCUT-ID [ACCOUNT] [-d|--days DAYS]`

Read aggregate shortcut statistics for a UTC window from 1 through 90 days. The default is 30.

## config

`suffix config`

Print active API base, selected profile, whether a key exists, and the configuration path without exposing a secret.

## man

`suffix man [show]`

Print this complete embedded manual.

`suffix man install [-d|--dir DIR]`

Install `suffix.1` using Somme's shared installer. Without `--dir`, standard Homebrew, local, user-local, and system section-1 directories are tried.

# COMPATIBILITY COMMANDS

The hidden `shortcuts` and `domains` namespaces preserve older API-shaped scripts. Prefer the public terse commands above.

`suffix shortcuts list`
: List shortcuts.

`suffix shortcuts create -d|--domain-id ID -t|--tail TAIL -u|--target-url URL [-n|--title TITLE] [-p|--photo] [-m|--photo-mode local|remote]`
: Create a shortcut.

`suffix shortcuts update -i|--id ID -v|--version VERSION -d|--domain-id ID -t|--tail TAIL -u|--target-url URL [-n|--title TITLE] [-a|--active BOOL] [-p|--photo] [-m|--photo-mode local|remote]`
: Replace API fields with optimistic concurrency.

`suffix shortcuts delete -i|--id ID -v|--version VERSION`
: Delete a shortcut.

`suffix domains list`
: List domains.

`suffix domains add -n|--hostname HOSTNAME`
: Add a domain.

`suffix domains delete -i|--id ID`
: Delete a domain.

# AUTHENTICATION AND ACCOUNTS

Browser login creates a one-time API key and returns it through a loopback callback on `127.0.0.1`. Keys are stored by named profile. Environment credentials override stored values. API keys authenticate only their bound account; an administrator key may bypass product quotas for its own inventory but does not gain cross-account authority.

# OUTPUT AND RATE LIMITS

Human list output is tab-separated. JSON, XML, and YAML are intended for automation. API failures go to standard error and produce nonzero status. Suffix consumes Somme-compatible rate-limit headers; Free and Pro exhaustion resets at the next UTC day, while server-verified administrators receive an explicit unlimited limit.

# ENVIRONMENT

`SUFFIX_API_BASE`
: Override the saved API base.

`SUFFIX_API_KEY`
: Override the saved bearer key.

`HOSTNAME`, `COMPUTERNAME`
: Used in the default dashboard key label.

# FILES

`suffix/config.toml`
: Named profiles below the platform configuration directory.

# EXAMPLES

```
suffix login person@example.com
suffix add -d pair.rs typesec https://example.com/type-security
suffix add -p -l https://example.com/public
suffix ls -l
suffix edit shortcut-id -u https://example.com/new -v 4
suffix photo shortcut-id -l
suffix upload -d pair.rs report ./report.pdf -p
suffix domain ls -j
suffix man install -d ~/.local/share/man/man1
```

# EXIT STATUS

Zero indicates success. Nonzero indicates parser validation, local I/O, login, authentication, API, optimistic-concurrency, quota, transfer, upload, or manual installation failure.

# SEE ALSO

`somme(1)`, `bay(1)`, <https://suffix.org>, and <https://github.com/firstpair/suffix>.
