use std::{
    collections::BTreeMap,
    fs,
    io::{self, IsTerminal, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use rand::{RngExt, distr::Alphanumeric};
use regex::RegexBuilder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

const DEFAULT_APP_URL: &str = "https://suffix.org";
const DEFAULT_API_BASE: &str = "https://suffix.org/api/v1";
const MANPAGE: &str = include_str!("../man/suffix.1");

#[derive(Parser)]
#[command(name = "suffix", version, about = "CLI client for suffix.org")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Select a saved account, or approve and save a new API key.
    Login(LoginArgs),
    /// Forget a saved account's API key but keep cached account metadata.
    Logout { account: String },
    /// List shortcuts.
    Ls(ListArgs),
    /// Search shortcut URLs and targets with a regular expression.
    Search(SearchArgs),
    /// Add a shortcut.
    Add(AddArgs),
    /// Edit an existing shortcut.
    Edit(EditArgs),
    /// Upload a Business file and create a shortcut for it.
    Upload(UploadArgs),
    /// Remove a shortcut.
    Rm(RemoveArgs),
    /// Choose whether a Photo shortcut serves its stored or original image.
    Photo(PhotoArgs),
    /// Add, change, or remove a stored-object password.
    Password(PasswordArgs),
    /// Move a domain between logged-in accounts.
    Mv(MoveArgs),
    /// Create a short-lived transfer code for a domain.
    Transfer(TransferArgs),
    /// Accept a domain transfer code into the active account.
    Accept(AcceptArgs),
    /// Manage domains.
    Domain(DomainArgs),
    /// List or switch stored accounts.
    Account(AccountArgs),
    /// Read aggregate shortcut statistics.
    Stats(StatsArgs),
    /// Show the active CLI configuration without printing secrets.
    Config,
    /// Install the Suffix manual page.
    Man(ManArgs),
    /// Manage shortcuts. Prefer `suffix ls`, `suffix add`, and `suffix rm`.
    #[command(hide = true)]
    Shortcuts(ShortcutsArgs),
    /// Manage domains. Prefer `suffix ls --domains`, `suffix add --domain`, and `suffix rm --domain`.
    #[command(hide = true)]
    Domains(DomainsArgs),
}

#[derive(Args)]
struct ManArgs {
    #[command(subcommand)]
    command: ManCommand,
}

#[derive(Subcommand)]
enum ManCommand {
    /// Install suffix(1) into a standard writable man1 directory.
    Install(ManInstallArgs),
}

#[derive(Args)]
struct ManInstallArgs {
    /// Specific man1 directory to install into.
    #[arg(long, value_name = "DIR")]
    dir: Option<PathBuf>,
}

#[derive(Args)]
struct LoginArgs {
    /// Email address to select or create a saved account profile for.
    login_email: Option<String>,
    /// Suffix dashboard URL to open.
    #[arg(long, default_value = DEFAULT_APP_URL)]
    app_url: String,
    /// Email address to hint in the browser sign-in flow.
    #[arg(long)]
    email: Option<String>,
    /// Local profile name to save this key under.
    #[arg(long)]
    account: Option<String>,
    /// API key name shown in the Suffix dashboard.
    #[arg(long)]
    name: Option<String>,
    /// Mint and replace the saved key instead of reusing a matching local key.
    #[arg(long)]
    renew: bool,
    /// Print the login URL instead of opening the browser.
    #[arg(long)]
    no_open: bool,
    /// Seconds to wait for browser approval.
    #[arg(long, default_value_t = 120)]
    timeout: u64,
}

#[derive(Args)]
struct ListArgs {
    /// Saved account email to query instead of the active account.
    account: Option<String>,
    /// List domains instead of shortcuts. Prefer `suffix domain ls`.
    #[arg(long, hide = true)]
    domains: bool,
    /// Emit formatted JSON.
    #[arg(long)]
    json: bool,
    /// Emit formatted XML.
    #[arg(long)]
    xml: bool,
    /// Emit formatted YAML.
    #[arg(long)]
    yaml: bool,
    /// Include visit counts after the target.
    #[arg(short = 'l', long = "stats")]
    stats: bool,
}

#[derive(Args)]
struct SearchArgs {
    /// Case-insensitive regular expression matched against short URLs and targets.
    pattern: String,
    /// Saved account email to query instead of the active account.
    account: Option<String>,
}

#[derive(Args)]
struct DomainListArgs {
    /// Saved account email to query instead of the active account.
    account: Option<String>,
    /// Emit formatted JSON.
    #[arg(long)]
    json: bool,
    /// Emit formatted XML.
    #[arg(long)]
    xml: bool,
    /// Emit formatted YAML.
    #[arg(long)]
    yaml: bool,
    /// Include aggregate visit counts for shortcuts on each domain.
    #[arg(short = 'l', long = "stats")]
    stats: bool,
}

#[derive(Args)]
struct AddArgs {
    /// Shortcut domain hostname. Example: `suffix add -d pair.rs typesec URL`.
    #[arg(short = 'd', long = "domain", value_name = "HOST")]
    domain: Option<String>,
    /// Use the public suf.cx domain. With no tail, choose from generated candidates.
    #[arg(long, conflicts_with_all = ["domain", "domain_id"])]
    public: bool,
    /// Include shortest letter-only candidates for --public.
    #[arg(long, requires = "public")]
    letters: bool,
    /// Include shortest letter-and-number candidates for --public.
    #[arg(long, requires = "public")]
    alphanumeric: bool,
    /// Include short dash-separated word candidates for --public.
    #[arg(long, requires = "public")]
    words: bool,
    /// Target URL, or `HOST/TAIL` when the target URL is passed next.
    value: String,
    /// Shortcut tail, or target URL when VALUE is `HOST/TAIL`.
    tail: Option<String>,
    /// Target URL when using `-d HOST TAIL URL`.
    target_url: Option<String>,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Domain ID for a new shortcut. Defaults to the first owned domain.
    #[arg(long)]
    domain_id: Option<String>,
    /// Optional shortcut title.
    #[arg(long)]
    title: Option<String>,
    /// Create a top-tier Photo shortcut instead of a redirect.
    #[arg(long)]
    photo: bool,
    /// Default Photo source. Local stores and serves a managed copy; remote uses the original.
    #[arg(long, value_enum, default_value_t = PhotoMode::Remote, requires = "photo")]
    photo_mode: PhotoMode,
    /// Create another shortcut even when this target already has one.
    #[arg(long)]
    allow_duplicate_target: bool,
    /// Replace an occupied tail's destination instead of failing.
    #[arg(long)]
    edit_existing: bool,
    /// Password-protect the stored photo. The remote target remains public.
    #[arg(long, requires = "photo")]
    protect: bool,
    /// Read the storage password from a file instead of prompting securely.
    #[arg(long, value_name = "PATH", requires = "photo")]
    password_file: Option<PathBuf>,
}

#[derive(Args)]
struct UploadArgs {
    /// Shortcut domain hostname. Example: `suffix upload -d pair.rs report ./report.pdf`.
    #[arg(short = 'd', long = "domain", value_name = "HOST")]
    domain: Option<String>,
    /// `HOST/TAIL`, or TAIL when --domain is supplied.
    value: String,
    /// Local file to upload.
    file: PathBuf,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Domain ID. Defaults to the first owned domain when no hostname is supplied.
    #[arg(long)]
    domain_id: Option<String>,
    /// Optional shortcut title.
    #[arg(long)]
    title: Option<String>,
    /// Password-protect the uploaded file.
    #[arg(long)]
    protect: bool,
    /// Read the storage password from a file instead of prompting securely.
    #[arg(long, value_name = "PATH")]
    password_file: Option<PathBuf>,
}

#[derive(Args)]
struct EditArgs {
    /// Shortcut ID.
    id: String,
    /// Change the tail.
    #[arg(long)]
    tail: Option<String>,
    /// Change the destination URL.
    #[arg(long)]
    target_url: Option<String>,
    /// Change the label. Pass an empty value to remove it.
    #[arg(long)]
    title: Option<String>,
    /// Pause or activate the shortcut.
    #[arg(long, value_name = "BOOL")]
    active: Option<bool>,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Shortcut version. If omitted, suffix looks it up first.
    #[arg(long)]
    version: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum PhotoMode {
    Local,
    Remote,
}

#[derive(Args)]
struct PhotoArgs {
    /// Photo shortcut ID.
    id: String,
    /// Serve the account-stored image by default.
    #[arg(
        short = 'l',
        long,
        conflicts_with_all = ["remote", "drop"],
        required_unless_present_any = ["remote", "drop"]
    )]
    local: bool,
    /// Resolve to the original image by default.
    #[arg(
        short = 'r',
        long,
        conflicts_with_all = ["local", "drop"],
        required_unless_present_any = ["local", "drop"]
    )]
    remote: bool,
    /// Remove the managed photo and return the shortcut to a normal redirect.
    #[arg(long, conflicts_with_all = ["local", "remote"])]
    drop: bool,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Shortcut version. If omitted, suffix looks it up first.
    #[arg(long)]
    version: Option<u64>,
}

#[derive(Args)]
struct PasswordArgs {
    /// Stored photo or managed-file shortcut ID.
    id: String,
    /// Add or change the password.
    #[arg(long, conflicts_with = "remove", required_unless_present = "remove")]
    add: bool,
    /// Remove password protection.
    #[arg(long, conflicts_with = "add", required_unless_present = "add")]
    remove: bool,
    /// Read the new password from a file instead of prompting securely.
    #[arg(long, value_name = "PATH", requires = "add")]
    password_file: Option<PathBuf>,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Shortcut version. If omitted, suffix looks it up first.
    #[arg(long)]
    version: Option<u64>,
}

#[derive(Args)]
struct RemoveArgs {
    /// Remove a domain instead of a shortcut. Prefer `suffix domain rm ID`.
    #[arg(long, hide = true)]
    domain: bool,
    /// Shortcut or domain ID.
    id: String,
    /// Saved account email to modify instead of the active account.
    account: Option<String>,
    /// Shortcut version. If omitted, suffix looks it up first.
    #[arg(long)]
    version: Option<u64>,
}

#[derive(Args)]
struct MoveArgs {
    /// Domain hostname or ID to move.
    domain: String,
    /// Other stored account name or email. With two values, this is the target.
    account: Option<String>,
    /// Explicit target account when supplying both source and target accounts.
    target: Option<String>,
    /// Move from the active account to ACCOUNT (the default for one account).
    #[arg(long, conflicts_with = "from")]
    to: bool,
    /// Move from ACCOUNT to the active account.
    #[arg(long, conflicts_with = "to")]
    from: bool,
    /// Skip the interactive confirmation prompt.
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct TransferArgs {
    /// Domain hostname or ID to transfer.
    domain: String,
    /// Optional receiving account email. If set, only that account can accept.
    #[arg(long, short = 't')]
    to: Option<String>,
    /// Minutes before the transfer code expires.
    #[arg(long, default_value_t = 15)]
    minutes: u16,
}

#[derive(Args)]
struct AcceptArgs {
    /// Transfer code from `suffix transfer`.
    code: String,
}

#[derive(Args)]
struct DomainArgs {
    #[command(subcommand)]
    command: DomainCliCommand,
}

#[derive(Subcommand)]
enum DomainCliCommand {
    /// List domains.
    Ls(DomainListArgs),
    /// Add a domain.
    Add {
        hostname: String,
        /// Saved account email to modify instead of the active account.
        account: Option<String>,
    },
    /// Remove an empty domain.
    Rm {
        id: String,
        /// Saved account email to modify instead of the active account.
        account: Option<String>,
    },
}

#[derive(Args)]
struct AccountArgs {
    /// Empty lists, NAME switches, `add NAME --key ...` stores, `rm NAME` forgets.
    args: Vec<String>,
    /// Show cached API base, counts, and refresh status.
    #[arg(short = 'l', long)]
    long: bool,
    /// Email address to cache for `suffix account add NAME --key ...`.
    #[arg(long)]
    email: Option<String>,
    /// API key for `suffix account add NAME --key ...`.
    #[arg(long)]
    key: Option<String>,
    /// API base for `suffix account add NAME --key ...`.
    #[arg(long, default_value = DEFAULT_API_BASE)]
    api_base: String,
}

#[derive(Args)]
struct ShortcutsArgs {
    #[command(subcommand)]
    command: ShortcutCommand,
}

#[derive(Subcommand)]
enum ShortcutCommand {
    /// List all shortcuts.
    List,
    /// Create a shortcut.
    Create {
        #[arg(long)]
        domain_id: String,
        #[arg(long)]
        tail: String,
        #[arg(long)]
        target_url: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        photo: bool,
        #[arg(long, value_enum, default_value_t = PhotoMode::Remote, requires = "photo")]
        photo_mode: PhotoMode,
    },
    /// Update a shortcut with optimistic concurrency.
    Update {
        #[arg(long)]
        id: String,
        #[arg(long)]
        version: u64,
        #[arg(long)]
        domain_id: String,
        #[arg(long)]
        tail: String,
        #[arg(long)]
        target_url: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value_t = true)]
        active: bool,
        #[arg(long)]
        photo: bool,
        #[arg(long, value_enum, default_value_t = PhotoMode::Remote, requires = "photo")]
        photo_mode: PhotoMode,
    },
    /// Delete a shortcut with optimistic concurrency.
    Delete {
        #[arg(long)]
        id: String,
        #[arg(long)]
        version: u64,
    },
}

#[derive(Args)]
struct DomainsArgs {
    #[command(subcommand)]
    command: DomainCommand,
}

#[derive(Subcommand)]
enum DomainCommand {
    /// List domains.
    List,
    /// Add a domain to the authenticated account.
    Add {
        #[arg(long)]
        hostname: String,
    },
    /// Delete an empty domain.
    Delete {
        #[arg(long)]
        id: String,
    },
}

#[derive(Args)]
struct StatsArgs {
    /// Shortcut ID to inspect.
    shortcut_id: String,
    /// Saved account email to query instead of the active account.
    account: Option<String>,
    /// UTC day window from 1 through 90.
    #[arg(long, default_value_t = 30)]
    days: u16,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Config {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    active_account: Option<String>,
    #[serde(default)]
    accounts: BTreeMap<String, AccountConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct AccountConfig {
    api_base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    domain_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visit_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_checked_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ShortcutRow {
    shortcut: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    visits: Option<u64>,
}

#[derive(Debug, Serialize)]
struct DomainRow {
    id: String,
    hostname: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owner_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vercel_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vercel_misconfigured: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visits: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
    Xml,
    Yaml,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => overview(),
        Some(Command::Login(args)) => login(args),
        Some(Command::Logout { account }) => logout(&account),
        Some(Command::Ls(args)) => {
            let config = load_config()?;
            let api = api_for_account(&config, args.account.as_deref())?;
            if args.domains {
                api.get("domains")
            } else {
                list_shortcuts(&api, args)
            }
        }
        Some(Command::Search(args)) => search_shortcuts(args),
        Some(Command::Add(args)) => add(args),
        Some(Command::Edit(args)) => edit_shortcut(args),
        Some(Command::Upload(args)) => upload_file(args),
        Some(Command::Rm(args)) => remove(args),
        Some(Command::Photo(args)) => configure_photo(args),
        Some(Command::Password(args)) => configure_password(args),
        Some(Command::Mv(args)) => move_domain(args),
        Some(Command::Transfer(args)) => transfer_code(args),
        Some(Command::Accept(args)) => accept_transfer(args),
        Some(Command::Domain(args)) => match args.command {
            DomainCliCommand::Ls(args) => {
                let config = load_config()?;
                let api = api_for_account(&config, args.account.as_deref())?;
                list_domains(&api, args)
            }
            DomainCliCommand::Add { hostname, account } => {
                let config = load_config()?;
                let api = api_for_account(&config, account.as_deref())?;
                api.post("domains", json!({ "hostname": hostname }))
            }
            DomainCliCommand::Rm { id, account } => {
                let config = load_config()?;
                let api = api_for_account(&config, account.as_deref())?;
                api.delete(&format!("domains?id={}", encode(&id)))
            }
        },
        Some(Command::Account(args)) => account(args),
        Some(Command::Config) => print_config(),
        Some(Command::Man(args)) => match args.command {
            ManCommand::Install(args) => install_manpage(args.dir),
        },
        Some(Command::Shortcuts(args)) => {
            let api = Api::from_config()?;
            match args.command {
                ShortcutCommand::List => api.get("shortcuts"),
                ShortcutCommand::Create { domain_id, tail, target_url, title, photo, photo_mode } => api.post(
                    "shortcuts",
                    shortcut_payload(json!({ "domainId": domain_id, "tail": tail, "targetUrl": target_url, "title": title }), photo, photo_mode),
                ),
                ShortcutCommand::Update { id, version, domain_id, tail, target_url, title, active, photo, photo_mode } => api.patch(
                    "shortcuts",
                    shortcut_payload(json!({
                        "id": id,
                        "version": version,
                        "domainId": domain_id,
                        "tail": tail,
                        "targetUrl": target_url,
                        "title": title,
                        "isActive": active,
                    }), photo, photo_mode),
                ),
                ShortcutCommand::Delete { id, version } => {
                    api.delete(&format!("shortcuts?id={}&version={}", encode(&id), version))
                }
            }
        }
        Some(Command::Domains(args)) => {
            let api = Api::from_config()?;
            match args.command {
                DomainCommand::List => api.get("domains"),
                DomainCommand::Add { hostname } => {
                    api.post("domains", json!({ "hostname": hostname }))
                }
                DomainCommand::Delete { id } => api.delete(&format!("domains?id={}", encode(&id))),
            }
        }
        Some(Command::Stats(args)) => {
            let config = load_config()?;
            let api = api_for_account(&config, args.account.as_deref())?;
            api.get(&format!(
                "stats?shortcutId={}&days={}",
                encode(&args.shortcut_id),
                args.days
            ))
        }
    }
}

fn overview() -> Result<()> {
    let api = Api::from_config()?;
    list_shortcuts(
        &api,
        ListArgs {
            account: None,
            domains: false,
            json: false,
            xml: false,
            yaml: false,
            stats: true,
        },
    )?;
    println!();
    println!("accounts");
    let config = load_config()?;
    print_accounts(&config, true);
    Ok(())
}

fn login(args: LoginArgs) -> Result<()> {
    let login_email = login_email_hint(&args)?;
    let mut config = load_config()?;
    let account = login_account_name(&config, args.account.as_deref(), login_email.as_deref())?;
    if !args.renew && saved_key_matches(&config, &account, login_email.as_deref()) {
        config.active_account = Some(account.clone());
        save_config(&config)?;
        println!(
            "Reusing saved {account} for {}",
            config.accounts[&account].api_base
        );
        return Ok(());
    }

    let listener =
        TcpListener::bind("127.0.0.1:0").context("could not bind the local login callback")?;
    listener
        .set_nonblocking(false)
        .context("could not configure the local login callback")?;
    let port = listener.local_addr()?.port();
    let state = random_state();
    let callback = format!("http://127.0.0.1:{port}/callback");
    let key_name = args.name.unwrap_or_else(|| default_key_name(&account));
    let login_url = build_login_url(
        &args.app_url,
        &callback,
        &state,
        &key_name,
        login_email.as_deref(),
    )?;

    println!("Opening {login_url}");
    if args.no_open {
        println!("Paste that URL into your browser to continue.");
    } else {
        open::that(&login_url).context("could not open the browser")?;
    }
    eprintln!("Waiting for browser approval...");

    listener
        .set_nonblocking(true)
        .context("could not configure the local login callback")?;
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(accepted) => break accepted,
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                bail!(
                    "timed out waiting for browser approval; run `suffix login --no-open` if the browser did not open the approval page"
                )
            }
            Err(error) => return Err(error).context("could not accept the login callback"),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .context("could not set callback read timeout")?;
    let mut buffer = [0_u8; 8192];
    let bytes = stream
        .read(&mut buffer)
        .context("could not read the browser callback")?;
    let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
    let callback = parse_callback(&request)?;
    if callback.state != state {
        respond(&mut stream, 400, "suffix login failed: state mismatch")?;
        bail!("browser callback state did not match the CLI login request");
    }
    if let Some(error) = callback.error {
        respond(&mut stream, 400, &format!("suffix login failed: {error}"))?;
        bail!("browser login failed: {error}");
    }
    let api_key = callback
        .key
        .ok_or_else(|| anyhow!("browser callback did not include an API key"))?;
    let api_base = callback
        .api_base
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());

    config.api_base = None;
    config.api_key = None;
    config.active_account = Some(account.clone());
    config.accounts.insert(
        account.clone(),
        AccountConfig {
            api_base: api_base.clone(),
            api_key: Some(api_key),
            email: login_email,
            domain_count: None,
            link_count: None,
            visit_count: None,
            last_checked_at: None,
            last_error: None,
        },
    );
    save_config(&config)?;
    respond(
        &mut stream,
        200,
        "suffix login complete. You can close this tab.",
    )?;
    println!("Saved {account} for {api_base}");
    Ok(())
}

fn add(args: AddArgs) -> Result<()> {
    let config = load_config()?;
    let api = api_for_account(&config, add_account(&args))?;
    let mut shortcut = shortcut_add_fields(&args)?;
    if args.public && shortcut.tail.is_empty() {
        shortcut.tail = select_public_tail(&api, &args)?;
    }
    let mut payload = shortcut_payload(
        json!({
            "tail": shortcut.tail,
            "targetUrl": shortcut.target_url,
            "title": args.title,
        }),
        args.photo,
        args.photo_mode,
    );
    apply_storage_password(
        &mut payload,
        args.protect || args.password_file.is_some(),
        args.password_file.as_deref(),
    )?;
    match shortcut.domain {
        ShortcutAddDomain::Hostname(hostname) => payload["domain"] = json!(hostname),
        ShortcutAddDomain::Id(domain_id) => payload["domainId"] = json!(domain_id),
        ShortcutAddDomain::Default => payload["domainId"] = json!(api.default_domain_id()?),
    }
    let shortcuts = api.shortcuts_payload()?;
    if let Some(existing) = occupied_tail(&shortcuts, &payload) {
        let short = shortcut_name(existing);
        eprintln!("Tail already taken: {short}");
        let edit = args.edit_existing
            || (io::stdin().is_terminal()
                && confirm("Edit that shortcut to use the new target?", false)?);
        if !edit {
            bail!("tail is already taken; rerun with --edit-existing to update it");
        }
        payload["id"] = existing.get("id").cloned().unwrap_or(Value::Null);
        payload["version"] = existing.get("version").cloned().unwrap_or(Value::Null);
        payload["isActive"] = existing.get("isActive").cloned().unwrap_or(json!(true));
        return api.patch("shortcuts", payload);
    }
    let duplicates = shortcuts
        .iter()
        .filter(|existing| {
            same_target(
                existing
                    .get("targetUrl")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                payload
                    .get("targetUrl")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            )
        })
        .collect::<Vec<_>>();
    if !duplicates.is_empty() && !args.allow_duplicate_target {
        eprintln!(
            "That target already has {} shortcut{}:",
            duplicates.len(),
            if duplicates.len() == 1 { "" } else { "s" }
        );
        for (index, existing) in duplicates.iter().enumerate() {
            eprintln!(
                "  {}. {}\t{}",
                index + 1,
                shortcut_name(existing),
                existing
                    .get("targetUrl")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
        }
        if !io::stdin().is_terminal() {
            bail!("reuse an existing shortcut or rerun with --allow-duplicate-target");
        }
        if confirm("Reuse the first existing shortcut?", true)? {
            println!(
                "{}\t{}",
                shortcut_name(duplicates[0]),
                duplicates[0]
                    .get("targetUrl")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            );
            return Ok(());
        }
        if !confirm("Create another shortcut for the same target?", false)? {
            bail!("shortcut creation canceled");
        }
    }
    api.post("shortcuts", payload)
}

fn search_shortcuts(args: SearchArgs) -> Result<()> {
    let expression = RegexBuilder::new(&args.pattern)
        .case_insensitive(true)
        .build()
        .with_context(|| format!("invalid regular expression: {}", args.pattern))?;
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    let shortcuts = api.shortcuts_payload()?;
    for shortcut in shortcuts {
        let short = shortcut_name(&shortcut);
        let target = shortcut
            .get("targetUrl")
            .and_then(Value::as_str)
            .unwrap_or("");
        let short_match = expression.is_match(&short);
        let target_match = expression.is_match(target);
        if short_match || target_match {
            let scope = match (short_match, target_match) {
                (true, true) => "shortcut+target",
                (true, false) => "shortcut",
                (false, true) => "target",
                _ => unreachable!(),
            };
            println!("{scope}\t{short}\t{target}");
        }
    }
    Ok(())
}

fn occupied_tail<'a>(shortcuts: &'a [Value], payload: &Value) -> Option<&'a Value> {
    let tail = payload.get("tail").and_then(Value::as_str)?;
    shortcuts.iter().find(|existing| {
        if !existing
            .get("tail")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case(tail))
        {
            return false;
        }
        if let Some(domain_id) = payload.get("domainId").and_then(Value::as_str) {
            return existing.get("domainId").and_then(Value::as_str) == Some(domain_id);
        }
        payload
            .get("domain")
            .and_then(Value::as_str)
            .is_some_and(|hostname| {
                existing
                    .get("hostname")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(hostname))
            })
    })
}

fn shortcut_name(value: &Value) -> String {
    format!(
        "{}/{}",
        value.get("hostname").and_then(Value::as_str).unwrap_or("?"),
        value.get("tail").and_then(Value::as_str).unwrap_or("?")
    )
}

fn same_target(left: &str, right: &str) -> bool {
    match (Url::parse(left), Url::parse(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

fn confirm(prompt: &str, default: bool) -> Result<bool> {
    eprint!("{prompt} {} ", if default { "[Y/n]" } else { "[y/N]" });
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        "" => Ok(default),
        _ => bail!("answer yes or no"),
    }
}

fn shortcut_payload(mut payload: Value, photo: bool, photo_mode: PhotoMode) -> Value {
    if photo {
        payload["type"] = json!("photo");
        payload["photoMode"] = json!(photo_mode);
    }
    payload
}

fn apply_storage_password(
    payload: &mut Value,
    protect: bool,
    password_file: Option<&Path>,
) -> Result<()> {
    if !protect {
        return Ok(());
    }
    let password = if let Some(path) = password_file {
        fs::read_to_string(path)
            .with_context(|| format!("could not read password file {}", path.display()))?
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    } else if io::stdin().is_terminal() {
        rpassword::prompt_password("Storage password: ")?
    } else {
        bail!("--protect requires an interactive terminal or --password-file PATH");
    };
    if !(8..=128).contains(&password.chars().count()) {
        bail!("storage password must be between 8 and 128 characters");
    }
    payload["protectStorage"] = json!(true);
    payload["storagePassword"] = json!(password);
    Ok(())
}

fn upload_file(args: UploadArgs) -> Result<()> {
    let metadata = fs::metadata(&args.file)
        .with_context(|| format!("could not read {}", args.file.display()))?;
    if !metadata.is_file() {
        bail!("{} is not a regular file", args.file.display());
    }
    if metadata.len() == 0 || metadata.len() > 100 * 1024 * 1024 {
        bail!("files must be between 1 byte and 100 MB");
    }
    let file_name = safe_upload_name(
        args.file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("file name must be valid Unicode"))?,
    );
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    let (domain, tail) = upload_locator(&args)?;
    let domain = match domain {
        ShortcutAddDomain::Default => ShortcutAddDomain::Id(api.default_domain_id()?),
        selected => selected,
    };
    let account_id = api.account_id()?;
    let pathname = format!(
        "files/suffix/{}/{}/{}",
        account_id,
        random_uuid(),
        file_name
    );
    let token = api.request_json(api.client.post(api.url("files")?).json(&json!({
        "type": "blob.generate-client-token",
        "payload": {
            "pathname": pathname,
            "multipart": false,
            "clientPayload": json!({ "size": metadata.len() }).to_string()
        }
    })))?;
    let client_token = token
        .get("clientToken")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Suffix did not return a file upload token"))?;
    let bytes =
        fs::read(&args.file).with_context(|| format!("could not read {}", args.file.display()))?;
    let content_type = file_content_type(&file_name);
    let upload_client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .context("could not build file upload client")?;
    let uploaded: Value = upload_client
        .put("https://vercel.com/api/blob/")
        .query(&[("pathname", pathname.as_str())])
        .bearer_auth(client_token)
        .header("x-api-version", "12")
        .header("x-vercel-blob-access", "private")
        .header("x-content-type", content_type)
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .context("file upload failed")?
        .error_for_status()
        .context("Blob rejected the file upload")?
        .json()
        .context("Blob returned an invalid upload response")?;
    let blob_url = uploaded
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Blob upload did not return a URL"))?;
    let download_url = uploaded
        .get("downloadUrl")
        .and_then(Value::as_str)
        .unwrap_or(blob_url);
    let mut payload = json!({
        "type": "file",
        "tail": tail,
        "targetUrl": download_url,
        "fileUrl": blob_url,
        "fileName": file_name,
        "title": args.title,
    });
    apply_storage_password(
        &mut payload,
        args.protect || args.password_file.is_some(),
        args.password_file.as_deref(),
    )?;
    match domain {
        ShortcutAddDomain::Hostname(hostname) => payload["domain"] = json!(hostname),
        ShortcutAddDomain::Id(domain_id) => payload["domainId"] = json!(domain_id),
        ShortcutAddDomain::Default => {
            unreachable!("default upload domain was resolved before upload")
        }
    }
    if let Err(error) = api.post("shortcuts", payload) {
        let _ = api.request_json(
            api.client
                .post(api.url("files")?)
                .json(&json!({ "action": "cleanup", "url": blob_url })),
        );
        return Err(error);
    }
    Ok(())
}

fn upload_locator(args: &UploadArgs) -> Result<(ShortcutAddDomain, String)> {
    if let Some(hostname) = &args.domain {
        return Ok((
            ShortcutAddDomain::Hostname(hostname.clone()),
            args.value.clone(),
        ));
    }
    if let Some((hostname, tail)) = args.value.split_once('/')
        && !hostname.is_empty()
        && !tail.is_empty()
        && !tail.contains('/')
    {
        return Ok((
            ShortcutAddDomain::Hostname(hostname.to_string()),
            tail.to_string(),
        ));
    }
    Ok((
        args.domain_id
            .clone()
            .map_or(ShortcutAddDomain::Default, ShortcutAddDomain::Id),
        args.value.clone(),
    ))
}

fn safe_upload_name(value: &str) -> String {
    let name: String = value
        .chars()
        .map(|ch| {
            if ch == '/' || ch == '\\' || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = name.trim();
    let safe = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "download"
    } else {
        trimmed
    };
    safe.chars().take(180).collect()
}

fn file_content_type(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "txt" | "md" | "csv" => "text/plain",
        "json" => "application/json",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

fn random_uuid() -> String {
    let value = format!("{:032x}", rand::random::<u128>());
    format!(
        "{}-{}-4{}-8{}-{}",
        &value[0..8],
        &value[8..12],
        &value[13..16],
        &value[17..20],
        &value[20..32]
    )
}

fn configure_photo(args: PhotoArgs) -> Result<()> {
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    let version = match args.version {
        Some(value) => value,
        None => api.shortcut_version(&args.id)?,
    };
    if args.drop {
        return api.patch(
            "shortcuts",
            json!({ "id": args.id, "version": version, "storePhoto": false, "serveLocalPhoto": false, "preservePhotoPage": false, "protectStorage": false }),
        );
    }
    let mode = if args.local {
        PhotoMode::Local
    } else {
        PhotoMode::Remote
    };
    api.patch(
        "shortcuts",
        json!({ "id": args.id, "version": version, "type": "photo", "photoMode": mode }),
    )
}

fn edit_shortcut(args: EditArgs) -> Result<()> {
    if args.tail.is_none()
        && args.target_url.is_none()
        && args.title.is_none()
        && args.active.is_none()
    {
        bail!("specify at least one of --tail, --target-url, --title, or --active");
    }
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    let version = args.version.unwrap_or(api.shortcut_version(&args.id)?);
    let mut payload = json!({ "id": args.id, "version": version });
    if let Some(value) = args.tail {
        payload["tail"] = json!(value);
    }
    if let Some(value) = args.target_url {
        payload["targetUrl"] = json!(value);
    }
    if let Some(value) = args.title {
        payload["title"] = json!(value);
    }
    if let Some(value) = args.active {
        payload["isActive"] = json!(value);
    }
    api.patch("shortcuts", payload)
}

fn configure_password(args: PasswordArgs) -> Result<()> {
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    let version = args.version.unwrap_or(api.shortcut_version(&args.id)?);
    let mut payload = json!({ "id": args.id, "version": version, "protectStorage": args.add });
    if args.add {
        apply_storage_password(&mut payload, true, args.password_file.as_deref())?;
    }
    api.patch("shortcuts", payload)
}

fn logout(account: &str) -> Result<()> {
    let mut config = load_config()?;
    let name = account_selector(&config, account)?;
    let Some(saved) = config.accounts.get_mut(&name) else {
        bail!("no stored account named {name}");
    };
    saved.api_key = None;
    saved.last_error = None;
    if config.active_account.as_deref() == Some(&name) {
        config.active_account = first_logged_in_account(&config).or_else(|| Some(name.clone()));
    }
    save_config(&config)?;
    println!("Logged out {name}");
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ShortcutAddFields {
    domain: ShortcutAddDomain,
    tail: String,
    target_url: String,
}

#[derive(Debug, PartialEq, Eq)]
enum ShortcutAddDomain {
    Default,
    Id(String),
    Hostname(String),
}

fn shortcut_add_fields(args: &AddArgs) -> Result<ShortcutAddFields> {
    if args.domain.is_some() && args.domain_id.is_some() {
        bail!("pass either -d/--domain or --domain-id, not both");
    }
    if args.public {
        if let Some(target_url) = args.tail.as_deref().filter(|value| looks_like_url(value)) {
            return Ok(ShortcutAddFields {
                domain: ShortcutAddDomain::Hostname("suf.cx".to_string()),
                tail: args.value.clone(),
                target_url: target_url.to_string(),
            });
        }
        if !looks_like_url(&args.value) {
            bail!(
                "usage: suffix add --public [--letters] [--alphanumeric] [--words] URL, or suffix add --public TAIL URL"
            );
        }
        return Ok(ShortcutAddFields {
            domain: ShortcutAddDomain::Hostname("suf.cx".to_string()),
            tail: String::new(),
            target_url: args.value.clone(),
        });
    }
    if let Some(hostname) = args.domain.as_deref() {
        let target_url = args
            .target_url
            .as_deref()
            .filter(|value| !looks_like_email(value))
            .or(args.tail.as_deref())
            .ok_or_else(|| anyhow!("usage: suffix add -d HOST TAIL URL"))?;
        return Ok(ShortcutAddFields {
            domain: ShortcutAddDomain::Hostname(hostname.to_string()),
            tail: args.value.clone(),
            target_url: target_url.to_string(),
        });
    }
    if args
        .target_url
        .as_deref()
        .is_some_and(|value| !looks_like_email(value))
    {
        bail!("usage: suffix add -d HOST TAIL URL or suffix add HOST/TAIL URL");
    }
    if let Some(target_url) = args.tail.as_deref()
        && !looks_like_url(&args.value)
        && looks_like_url(target_url)
    {
        let (hostname, tail) = split_shortcut_locator(&args.value)?;
        return Ok(ShortcutAddFields {
            domain: ShortcutAddDomain::Hostname(hostname),
            tail,
            target_url: target_url.to_string(),
        });
    }
    Ok(ShortcutAddFields {
        domain: args
            .domain_id
            .clone()
            .map(ShortcutAddDomain::Id)
            .unwrap_or(ShortcutAddDomain::Default),
        tail: args
            .tail
            .clone()
            .unwrap_or_else(|| derive_tail(&args.value)),
        target_url: args.value.clone(),
    })
}

fn select_public_tail(api: &Api, args: &AddArgs) -> Result<String> {
    let mut styles = Vec::new();
    if args.letters || (!args.alphanumeric && !args.words) {
        styles.push("letters");
    }
    if args.alphanumeric {
        styles.push("alphanumeric");
    }
    if args.words {
        styles.push("words");
    }
    let payload = api.request_json(
        api.client
            .get(api.url(&format!("candidates?styles={}", styles.join(",")))?),
    )?;
    let groups = payload
        .get("candidates")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("suffix.org did not return public suffix candidates"))?;
    let candidates = styles
        .iter()
        .flat_map(|style| {
            groups
                .get(*style)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        bail!("suffix.org did not find an available suf.cx suffix");
    }
    if !io::stdin().is_terminal() {
        return Ok(candidates[0].clone());
    }
    eprintln!("Available suf.cx suffixes:");
    for (index, candidate) in candidates.iter().enumerate() {
        eprintln!("  {}. {}", index + 1, candidate);
    }
    eprint!("Choose [1]: ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let choice = if answer.trim().is_empty() {
        1
    } else {
        answer
            .trim()
            .parse::<usize>()
            .context("enter a candidate number")?
    };
    candidates
        .get(choice.saturating_sub(1))
        .cloned()
        .ok_or_else(|| anyhow!("candidate number is out of range"))
}

fn add_account(args: &AddArgs) -> Option<&str> {
    args.account.as_deref().or_else(|| {
        args.target_url
            .as_deref()
            .filter(|value| looks_like_email(value))
    })
}

fn split_shortcut_locator(value: &str) -> Result<(String, String)> {
    let (hostname, tail) = value
        .split_once('/')
        .ok_or_else(|| anyhow!("shortcut must be HOST/TAIL"))?;
    if hostname.is_empty() || tail.is_empty() || tail.contains('/') {
        bail!("shortcut must be HOST/TAIL");
    }
    Ok((hostname.to_string(), tail.to_string()))
}

fn looks_like_url(value: &str) -> bool {
    Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn looks_like_email(value: &str) -> bool {
    normalize_login_email(value).is_ok()
}

fn list_shortcuts(api: &Api, args: ListArgs) -> Result<()> {
    let payload = api.request_json(api.client.get(api.url("shortcuts")?))?;
    let rows = shortcut_rows(&payload, args.stats)?;
    match output_format(args.json, args.xml, args.yaml)? {
        OutputFormat::Text => print_shortcut_table(&rows),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Yaml => print!("{}", shortcuts_yaml(&rows)),
        OutputFormat::Xml => print!("{}", shortcuts_xml(&rows)),
    }
    Ok(())
}

fn list_domains(api: &Api, args: DomainListArgs) -> Result<()> {
    let payload = api.request_json(api.client.get(api.url("domains")?))?;
    let visits = if args.stats {
        let shortcuts = api.request_json(api.client.get(api.url("shortcuts")?))?;
        Some(domain_visit_counts(&shortcuts)?)
    } else {
        None
    };
    let rows = domain_rows(&payload, visits.as_ref())?;
    match output_format(args.json, args.xml, args.yaml)? {
        OutputFormat::Text => print_domain_table(&rows),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
        OutputFormat::Yaml => print!("{}", domains_yaml(&rows)),
        OutputFormat::Xml => print!("{}", domains_xml(&rows)),
    }
    Ok(())
}

fn remove(args: RemoveArgs) -> Result<()> {
    let config = load_config()?;
    let api = api_for_account(&config, args.account.as_deref())?;
    if args.domain {
        return api.delete(&format!("domains?id={}", encode(&args.id)));
    }
    let version = match args.version {
        Some(value) => value,
        None => api.shortcut_version(&args.id)?,
    };
    api.delete(&format!(
        "shortcuts?id={}&version={version}",
        encode(&args.id)
    ))
}

fn move_domain(args: MoveArgs) -> Result<()> {
    let config = load_config()?;
    let (from_name, to_name) = move_account_names(&config, &args)?;
    if from_name == to_name {
        bail!("source and target accounts are the same");
    }
    let from = config
        .accounts
        .get(&from_name)
        .ok_or_else(|| anyhow!("no stored account matching {from_name}"))?;
    let to = config
        .accounts
        .get(&to_name)
        .ok_or_else(|| anyhow!("no stored account matching {to_name}"))?;
    let target_key = to.api_key.as_deref().ok_or_else(|| {
        anyhow!(
            "{} is logged out; run `suffix login --account {to_name}`",
            account_display_email(&to_name, to)
        )
    })?;
    let from_label = account_display_email(&from_name, from);
    let to_label = account_display_email(&to_name, to);
    if !args.yes {
        confirm_domain_move(&args.domain, &from_label, &to_label)?;
    }
    let api = Api::from_account(&from_name, from)?;
    api.patch(
        "domains",
        json!({
            "action": "transfer",
            "domain": args.domain,
            "targetApiKey": target_key,
        }),
    )
}

fn move_account_names(config: &Config, args: &MoveArgs) -> Result<(String, String)> {
    let active = active_account(config)
        .map(|(name, _)| name.to_string())
        .ok_or_else(|| anyhow!("no active account; run `suffix login EMAIL` first"))?;
    let account = args.account.as_deref().ok_or_else(|| {
        anyhow!(
            "pass an account: `suffix mv DOMAIN TARGET_EMAIL` or `suffix mv --from DOMAIN SOURCE_EMAIL`"
        )
    })?;
    if let Some(target) = args.target.as_deref() {
        if args.from || args.to {
            bail!("--from and --to only apply when one account is supplied");
        }
        return Ok((
            account_selector(config, account)?,
            account_selector(config, target)?,
        ));
    }
    let other = account_selector(config, account)?;
    if args.from {
        Ok((other, active))
    } else {
        Ok((active, other))
    }
}

fn transfer_code(args: TransferArgs) -> Result<()> {
    let api = Api::from_config()?;
    let payload = api.request_json(api.client.post(api.url("transfers")?).json(&json!({
        "domain": args.domain,
        "targetEmail": args.to,
        "lifetimeMinutes": args.minutes,
    })))?;
    let code = payload
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("suffix.org did not return a transfer code"))?;
    let expires_at = payload
        .get("transfer")
        .and_then(|transfer| transfer.get("expiresAt"))
        .and_then(Value::as_str)
        .unwrap_or("");
    println!("Transfer code: {code}");
    if !expires_at.is_empty() {
        println!("Expires: {expires_at}");
    }
    println!("Give this code to the receiving account, then run: suffix accept {code}");
    Ok(())
}

fn accept_transfer(args: AcceptArgs) -> Result<()> {
    let api = Api::from_config()?;
    let payload = api.request_json(api.client.patch(api.url("transfers")?).json(&json!({
        "code": args.code,
    })))?;
    let domain = payload
        .get("domain")
        .and_then(|domain| domain.get("hostname"))
        .and_then(Value::as_str)
        .unwrap_or("domain");
    println!("Accepted transfer for {domain}.");
    Ok(())
}

fn confirm_domain_move(domain: &str, from: &str, to: &str) -> Result<()> {
    eprintln!("Move {domain} from {from} to {to}?");
    eprint!("Type `move {domain}` to confirm: ");
    io::stderr().flush().ok();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("could not read confirmation")?;
    if input.trim() != format!("move {domain}") {
        bail!("domain move cancelled");
    }
    Ok(())
}

fn account(args: AccountArgs) -> Result<()> {
    match args.args.as_slice() {
        [] => {
            let config = load_config()?;
            print_accounts(&config, args.long);
        }
        [action] if action == "ls" => {
            let config = load_config()?;
            print_accounts(&config, args.long);
        }
        [name] => {
            let mut config = load_config()?;
            let name = account_selector(&config, name)?;
            if !config.accounts.contains_key(&name) {
                bail!("no stored account named {name}");
            }
            config.active_account = Some(name.clone());
            save_config(&config)?;
            println!("Switched to {name}");
        }
        [action, name] if action == "add" => {
            let key = args
                .key
                .ok_or_else(|| anyhow!("pass --key when adding an account manually"))?;
            let name = normalize_account_name(name)?;
            let mut config = load_config()?;
            config.api_base = None;
            config.api_key = None;
            config.accounts.insert(
                name.clone(),
                AccountConfig {
                    api_base: args.api_base.trim_end_matches('/').to_string(),
                    api_key: Some(key),
                    email: args
                        .email
                        .as_deref()
                        .map(normalize_login_email)
                        .transpose()?,
                    domain_count: None,
                    link_count: None,
                    visit_count: None,
                    last_checked_at: None,
                    last_error: None,
                },
            );
            config.active_account = Some(name.clone());
            save_config(&config)?;
            println!("Switched to {name}");
        }
        [action, name] if action == "rm" => {
            let mut config = load_config()?;
            let name = account_selector(&config, name)?;
            if config.accounts.remove(&name).is_none() {
                bail!("no stored account named {name}");
            }
            if config.active_account.as_deref() == Some(&name) {
                config.active_account = config.accounts.keys().next().cloned();
            }
            save_config(&config)?;
            println!("Removed {name}");
        }
        _ => {
            bail!(
                "usage: suffix account [ls] [--long] | suffix account NAME | suffix account add NAME --key KEY [--email EMAIL] | suffix account rm NAME"
            )
        }
    }
    Ok(())
}

fn build_login_url(
    app_url: &str,
    callback: &str,
    state: &str,
    name: &str,
    email: Option<&str>,
) -> Result<String> {
    let mut url = Url::parse(app_url).context("app URL must be absolute")?;
    let mut query = url.query_pairs_mut();
    query
        .append_pair("cli_auth", "1")
        .append_pair("callback", callback)
        .append_pair("state", state)
        .append_pair("name", name);
    if let Some(email) = email {
        query.append_pair("email", email);
    }
    drop(query);
    Ok(url.to_string())
}

fn login_email_hint(args: &LoginArgs) -> Result<Option<String>> {
    match (&args.login_email, &args.email) {
        (Some(_), Some(_)) => {
            bail!("pass the login email either positionally or with --email, not both")
        }
        (Some(value), None) | (None, Some(value)) => Ok(Some(normalize_login_email(value)?)),
        (None, None) => Ok(None),
    }
}

fn login_account_name(
    config: &Config,
    account_input: Option<&str>,
    email: Option<&str>,
) -> Result<String> {
    if let Some(account) = account_input {
        return normalize_account_name(account);
    }
    if let Some(email) = email {
        if let Some((name, _)) = config.accounts.iter().find(|(_, account)| {
            account
                .email
                .as_deref()
                .map(|saved| saved.eq_ignore_ascii_case(email))
                .unwrap_or(false)
        }) {
            return Ok(name.clone());
        }
        return normalize_account_name(email);
    }
    if let Some((name, _)) = active_account(config) {
        return Ok(name.to_string());
    }
    bail!("pass an email or --account when logging in for the first time")
}

fn saved_key_matches(config: &Config, account: &str, email: Option<&str>) -> bool {
    let Some(saved) = config.accounts.get(account) else {
        return false;
    };
    if saved.api_key.is_none() {
        return false;
    }
    match (email, saved.email.as_deref()) {
        (Some(expected), Some(actual)) => actual.eq_ignore_ascii_case(expected),
        (Some(_), None) => false,
        (None, _) => true,
    }
}

fn normalize_login_email(value: &str) -> Result<String> {
    let email = value.trim().to_lowercase();
    if !email.contains('@')
        || email.starts_with('@')
        || email.ends_with('@')
        || email.contains(char::is_whitespace)
    {
        bail!("login email must be an email address");
    }
    Ok(email)
}

fn parse_callback(request: &str) -> Result<LoginCallback> {
    let first_line = request.lines().next().unwrap_or_default();
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow!("invalid browser callback"))?;
    let url =
        Url::parse(&format!("http://127.0.0.1{path}")).context("invalid browser callback URL")?;
    let mut callback = LoginCallback::default();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "state" => callback.state = value.into_owned(),
            "key" => callback.key = Some(value.into_owned()),
            "api_base" => callback.api_base = Some(value.into_owned()),
            "error" => callback.error = Some(value.into_owned()),
            _ => {}
        }
    }
    Ok(callback)
}

#[derive(Default)]
struct LoginCallback {
    state: String,
    key: Option<String>,
    api_base: Option<String>,
    error: Option<String>,
}

fn respond(stream: &mut impl Write, status: u16, message: &str) -> Result<()> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!("<!doctype html><title>suffix</title><main><h1>{escaped}</h1></main>");
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

struct Api {
    client: Client,
    base: String,
    key: String,
    account: String,
}

impl Api {
    fn from_config() -> Result<Self> {
        let config = load_config()?;
        let base = std::env::var("SUFFIX_API_BASE")
            .ok()
            .or_else(|| active_account(&config).map(|(_, account)| account.api_base.clone()))
            .or(config.api_base.clone())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        let key = std::env::var("SUFFIX_API_KEY")
            .ok()
            .or_else(|| active_account(&config).and_then(|(_, account)| account.api_key.clone()))
            .or(config.api_key.clone())
            .ok_or_else(|| {
                anyhow!("not logged in; run `suffix login --account NAME` or set SUFFIX_API_KEY")
            })?;
        let account = active_account(&config)
            .map(|(name, _)| name.to_string())
            .unwrap_or_else(|| "env".to_string());
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("could not build HTTP client")?,
            base: base.trim_end_matches('/').to_string(),
            key,
            account,
        })
    }

    fn from_account(name: &str, account: &AccountConfig) -> Result<Self> {
        let key = account.api_key.clone().ok_or_else(|| {
            anyhow!(
                "{} is logged out; run `suffix login --account {name}`",
                account_display_email(name, account)
            )
        })?;
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("could not build HTTP client")?,
            base: account.api_base.trim_end_matches('/').to_string(),
            key,
            account: name.to_string(),
        })
    }

    fn get(&self, resource: &str) -> Result<()> {
        self.send(self.client.get(self.url(resource)?))
    }

    fn post(&self, resource: &str, body: Value) -> Result<()> {
        self.send(self.client.post(self.url(resource)?).json(&body))
    }

    fn patch(&self, resource: &str, body: Value) -> Result<()> {
        self.send(self.client.patch(self.url(resource)?).json(&body))
    }

    fn delete(&self, resource: &str) -> Result<()> {
        self.send(self.client.delete(self.url(resource)?))
    }

    fn url(&self, resource: &str) -> Result<String> {
        let (name, query) = resource
            .split_once('?')
            .map_or((resource, ""), |(name, query)| (name, query));
        let route = route_resource(name)?;
        let extra = if query.is_empty() {
            String::new()
        } else {
            format!("&{query}")
        };
        Ok(format!("{}?resource={}{}", self.base, route, extra))
    }

    fn send(&self, request: reqwest::blocking::RequestBuilder) -> Result<()> {
        let payload = self.request_json(request)?;
        println!("{}", serde_json::to_string_pretty(&payload)?);
        Ok(())
    }

    fn request_json(&self, request: reqwest::blocking::RequestBuilder) -> Result<Value> {
        somme_cli::authenticated_json(request, &self.key)
    }

    fn default_domain_id(&self) -> Result<String> {
        let payload = self.request_json(self.client.get(self.url("domains")?))?;
        let domains = payload
            .get("domains")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("suffix.org did not return a domain list"))?;
        if domains.is_empty() {
            bail!(
                "no domains found for {}; use `suffix add --public URL` for a public suf.cx link, or add one with `suffix add --domain HOST`",
                self.account
            )
        }
        if domains.len() > 1 {
            eprintln!("Using first domain; pass --domain-id to choose another.");
        }
        domains[0]
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("suffix.org returned a domain without an id"))
    }

    fn account_id(&self) -> Result<String> {
        let payload = self.request_json(self.client.get(self.url("domains")?))?;
        payload
            .get("domains")
            .and_then(Value::as_array)
            .and_then(|domains| domains.first())
            .and_then(|domain| domain.get("ownerAccountId"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("no owned domain was available to identify the Suffix account"))
    }

    fn shortcuts_payload(&self) -> Result<Vec<Value>> {
        let payload = self.request_json(self.client.get(self.url("shortcuts")?))?;
        payload
            .get("shortcuts")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| anyhow!("suffix.org did not return a shortcut list"))
    }

    fn shortcut_version(&self, id: &str) -> Result<u64> {
        let payload = self.request_json(self.client.get(self.url("shortcuts")?))?;
        let shortcuts = payload
            .get("shortcuts")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("suffix.org did not return a shortcut list"))?;
        let shortcut = shortcuts
            .iter()
            .find(|shortcut| shortcut.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| anyhow!("shortcut {id} was not found for {}", self.account))?;
        shortcut
            .get("version")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("shortcut {id} did not include a version"))
    }
}

fn route_resource(resource: &str) -> Result<&'static str> {
    let name = resource.split('?').next().unwrap_or(resource);
    match name {
        "shortcuts" => Ok("shortcuts"),
        "domains" => Ok("domains"),
        "transfers" => Ok("transfers"),
        "stats" => Ok("stats"),
        "files" => Ok("files"),
        "candidates" => Ok("candidates"),
        _ => bail!("unsupported API resource {name}"),
    }
}

fn load_config() -> Result<Config> {
    let path = config_path()?;
    let mut config: Config = somme_cli::read_toml(&path)?;
    if discard_legacy_profiles(&mut config) {
        somme_cli::write_toml(&path, &config)?;
    }
    Ok(config)
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    somme_cli::write_toml(&path, config)
}

fn print_config() -> Result<()> {
    let config = load_config()?;
    let base = std::env::var("SUFFIX_API_BASE")
        .ok()
        .or_else(|| active_account(&config).map(|(_, account)| account.api_base.clone()))
        .or(config.api_base.clone())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let has_key = std::env::var("SUFFIX_API_KEY").is_ok()
        || active_account(&config)
            .and_then(|(_, account)| account.api_key.as_ref())
            .is_some()
        || config.api_key.is_some();
    println!("api_base = {base}");
    println!(
        "account = {}",
        active_account(&config)
            .map(|(name, _)| name)
            .unwrap_or("none")
    );
    println!(
        "api_key = {}",
        if has_key { "configured" } else { "missing" }
    );
    println!("config_file = {}", config_path()?.display());
    Ok(())
}

fn config_path() -> Result<PathBuf> {
    somme_cli::config_path("suffix")
}

fn random_state() -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn default_key_name(account: &str) -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "local machine".to_string());
    format!("suffix CLI {account} on {host}")
}

fn encode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

fn active_account(config: &Config) -> Option<(&str, &AccountConfig)> {
    if let Some(name) = config.active_account.as_deref()
        && let Some(account) = config.accounts.get(name)
    {
        return Some((name, account));
    }
    config
        .accounts
        .iter()
        .next()
        .map(|(name, account)| (name.as_str(), account))
}

fn api_for_account(config: &Config, selector: Option<&str>) -> Result<Api> {
    let Some(selector) = selector else {
        return Api::from_config();
    };
    let name = account_selector(config, selector)?;
    let account = config
        .accounts
        .get(&name)
        .ok_or_else(|| anyhow!("no stored account matching {selector}"))?;
    Api::from_account(&name, account)
}

fn first_logged_in_account(config: &Config) -> Option<String> {
    config
        .accounts
        .iter()
        .find(|(_, account)| account.api_key.is_some())
        .map(|(name, _)| name.clone())
}

fn discard_legacy_profiles(config: &mut Config) -> bool {
    let mut changed = config.api_base.take().is_some();
    changed |= config.api_key.take().is_some();
    changed |= config.accounts.remove("default").is_some();
    if config.active_account.as_deref() == Some("default") {
        config.active_account = config.accounts.keys().next().cloned();
        changed = true;
    }
    changed
}

fn print_accounts(config: &Config, long: bool) {
    if config.accounts.is_empty() {
        println!("No accounts. Run `suffix login EMAIL` or `suffix login --account NAME`.");
        return;
    }
    let active = active_account(config).map(|(name, _)| name);
    for (name, account) in &config.accounts {
        let marker = if Some(name.as_str()) == active {
            "*"
        } else {
            " "
        };
        let login_state = if account.api_key.is_some() {
            "logged in"
        } else {
            "logged out"
        };
        println!(
            "{marker} {}\t{login_state}",
            account_display_email(name, account)
        );
        if long {
            println!("  name\t{name}");
            println!("  api_base\t{}", account.api_base);
            if let Some(domain_count) = account.domain_count {
                println!("  domains\t{domain_count}");
            }
            if let Some(link_count) = account.link_count {
                println!("  links\t{link_count}");
            }
            if let Some(visit_count) = account.visit_count {
                println!("  visits\t{visit_count}");
            }
            if let Some(last_checked_at) = &account.last_checked_at {
                println!("  checked_at\t{last_checked_at}");
            }
            if let Some(last_error) = &account.last_error {
                println!("  last_error\t{last_error}");
            }
        }
    }
}

fn account_display_email(name: &str, account: &AccountConfig) -> String {
    account.email.clone().unwrap_or_else(|| name.to_string())
}

fn account_selector(config: &Config, selector: &str) -> Result<String> {
    let normalized = selector.trim().to_lowercase();
    if config.accounts.contains_key(selector) {
        return Ok(selector.to_string());
    }
    let matches: Vec<_> = config
        .accounts
        .iter()
        .filter(|(name, account)| {
            name.eq_ignore_ascii_case(&normalized)
                || account
                    .email
                    .as_deref()
                    .map(|email| email.eq_ignore_ascii_case(&normalized))
                    .unwrap_or(false)
        })
        .map(|(name, _)| name.clone())
        .collect();
    match matches.as_slice() {
        [name] => Ok(name.clone()),
        [] if normalized.contains('@') => bail!("no stored account matching {selector}"),
        [] => Ok(normalize_account_name(selector)?),
        _ => bail!("multiple stored accounts match {selector}"),
    }
}

fn normalize_account_name(value: &str) -> Result<String> {
    let name = value.trim();
    if name.is_empty() {
        bail!("account name cannot be empty");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@'))
    {
        bail!("account names may use letters, numbers, dash, underscore, dot, or @");
    }
    Ok(name.to_string())
}

fn derive_tail(target_url: &str) -> String {
    Url::parse(target_url)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back().map(str::to_string))
        })
        .filter(|tail| !tail.is_empty())
        .unwrap_or_else(|| "link".to_string())
}

fn output_format(json: bool, xml: bool, yaml: bool) -> Result<OutputFormat> {
    let requested = [json, xml, yaml]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();
    if requested > 1 {
        bail!("choose only one of --json, --xml, or --yaml");
    }
    if json {
        Ok(OutputFormat::Json)
    } else if xml {
        Ok(OutputFormat::Xml)
    } else if yaml {
        Ok(OutputFormat::Yaml)
    } else {
        Ok(OutputFormat::Text)
    }
}

fn shortcut_rows(payload: &Value, include_stats: bool) -> Result<Vec<ShortcutRow>> {
    let shortcuts = payload
        .get("shortcuts")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("suffix.org did not return a shortcut list"))?;
    shortcuts
        .iter()
        .map(|shortcut| {
            let hostname = string_field(shortcut, "hostname")?;
            let tail = string_field(shortcut, "tail")?;
            let target = string_field(shortcut, "targetUrl")?;
            Ok(ShortcutRow {
                shortcut: format_shortcut(&hostname, &tail),
                target,
                visits: if include_stats {
                    Some(
                        shortcut
                            .get("clickCount")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    )
                } else {
                    None
                },
            })
        })
        .collect()
}

fn domain_rows(
    payload: &Value,
    visits_by_domain: Option<&BTreeMap<String, u64>>,
) -> Result<Vec<DomainRow>> {
    let domains = payload
        .get("domains")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("suffix.org did not return a domain list"))?;
    domains
        .iter()
        .map(|domain| {
            let id = string_field(domain, "id")?;
            Ok(DomainRow {
                visits: visits_by_domain.map(|visits| visits.get(&id).copied().unwrap_or(0)),
                id,
                hostname: string_field(domain, "hostname")?,
                status: optional_string_field(domain, "status")
                    .unwrap_or_else(|| "unknown".to_string()),
                owner_name: optional_string_field(domain, "ownerName"),
                owner_email: optional_string_field(domain, "ownerEmail"),
                vercel_verified: optional_bool_field(domain, "vercelVerified"),
                vercel_misconfigured: optional_bool_field(domain, "vercelMisconfigured"),
                last_checked_at: optional_string_field(domain, "lastCheckedAt"),
                last_error: optional_string_field(domain, "lastError"),
            })
        })
        .collect()
}

fn domain_visit_counts(payload: &Value) -> Result<BTreeMap<String, u64>> {
    let shortcuts = payload
        .get("shortcuts")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("suffix.org did not return a shortcut list"))?;
    let mut visits = BTreeMap::new();
    for shortcut in shortcuts {
        let Some(domain_id) = shortcut.get("domainId").and_then(Value::as_str) else {
            continue;
        };
        let count = shortcut
            .get("clickCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        *visits.entry(domain_id.to_string()).or_insert(0) += count;
    }
    Ok(visits)
}

fn string_field(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("suffix.org returned a record without {key}"))
}

fn optional_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn optional_bool_field(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn format_shortcut(hostname: &str, tail: &str) -> String {
    if tail.is_empty() {
        hostname.to_string()
    } else {
        format!("{hostname}/{tail}")
    }
}

fn print_shortcut_table(rows: &[ShortcutRow]) {
    for row in rows {
        match row.visits {
            Some(visits) => println!("{}\t{}\t{}", row.shortcut, row.target, visits),
            None => println!("{}\t{}", row.shortcut, row.target),
        }
    }
}

fn print_domain_table(rows: &[DomainRow]) {
    for row in rows {
        match row.visits {
            Some(visits) => println!("{}\t{}\t{}", row.hostname, row.status, visits),
            None => println!("{}\t{}", row.hostname, row.status),
        }
    }
}

fn shortcuts_xml(rows: &[ShortcutRow]) -> String {
    let mut output = String::from("<shortcuts>\n");
    for row in rows {
        output.push_str("  <shortcut>\n");
        output.push_str(&format!(
            "    <short>{}</short>\n",
            xml_escape(&row.shortcut)
        ));
        output.push_str(&format!(
            "    <target>{}</target>\n",
            xml_escape(&row.target)
        ));
        if let Some(visits) = row.visits {
            output.push_str(&format!("    <visits>{visits}</visits>\n"));
        }
        output.push_str("  </shortcut>\n");
    }
    output.push_str("</shortcuts>\n");
    output
}

fn domains_xml(rows: &[DomainRow]) -> String {
    let mut output = String::from("<domains>\n");
    for row in rows {
        output.push_str("  <domain>\n");
        output.push_str(&format!("    <id>{}</id>\n", xml_escape(&row.id)));
        output.push_str(&format!(
            "    <hostname>{}</hostname>\n",
            xml_escape(&row.hostname)
        ));
        output.push_str(&format!(
            "    <status>{}</status>\n",
            xml_escape(&row.status)
        ));
        if let Some(owner_name) = &row.owner_name {
            output.push_str(&format!(
                "    <owner_name>{}</owner_name>\n",
                xml_escape(owner_name)
            ));
        }
        if let Some(owner_email) = &row.owner_email {
            output.push_str(&format!(
                "    <owner_email>{}</owner_email>\n",
                xml_escape(owner_email)
            ));
        }
        if let Some(vercel_verified) = row.vercel_verified {
            output.push_str(&format!(
                "    <vercel_verified>{vercel_verified}</vercel_verified>\n"
            ));
        }
        if let Some(vercel_misconfigured) = row.vercel_misconfigured {
            output.push_str(&format!(
                "    <vercel_misconfigured>{vercel_misconfigured}</vercel_misconfigured>\n"
            ));
        }
        if let Some(last_checked_at) = &row.last_checked_at {
            output.push_str(&format!(
                "    <last_checked_at>{}</last_checked_at>\n",
                xml_escape(last_checked_at)
            ));
        }
        if let Some(last_error) = &row.last_error {
            output.push_str(&format!(
                "    <last_error>{}</last_error>\n",
                xml_escape(last_error)
            ));
        }
        if let Some(visits) = row.visits {
            output.push_str(&format!("    <visits>{visits}</visits>\n"));
        }
        output.push_str("  </domain>\n");
    }
    output.push_str("</domains>\n");
    output
}

fn shortcuts_yaml(rows: &[ShortcutRow]) -> String {
    let mut output = String::new();
    for row in rows {
        output.push_str("- shortcut: ");
        output.push_str(&yaml_scalar(&row.shortcut));
        output.push('\n');
        output.push_str("  target: ");
        output.push_str(&yaml_scalar(&row.target));
        output.push('\n');
        if let Some(visits) = row.visits {
            output.push_str(&format!("  visits: {visits}\n"));
        }
    }
    output
}

fn domains_yaml(rows: &[DomainRow]) -> String {
    let mut output = String::new();
    for row in rows {
        output.push_str("- id: ");
        output.push_str(&yaml_scalar(&row.id));
        output.push('\n');
        output.push_str("  hostname: ");
        output.push_str(&yaml_scalar(&row.hostname));
        output.push('\n');
        output.push_str("  status: ");
        output.push_str(&yaml_scalar(&row.status));
        output.push('\n');
        if let Some(owner_name) = &row.owner_name {
            output.push_str("  owner_name: ");
            output.push_str(&yaml_scalar(owner_name));
            output.push('\n');
        }
        if let Some(owner_email) = &row.owner_email {
            output.push_str("  owner_email: ");
            output.push_str(&yaml_scalar(owner_email));
            output.push('\n');
        }
        if let Some(vercel_verified) = row.vercel_verified {
            output.push_str(&format!("  vercel_verified: {vercel_verified}\n"));
        }
        if let Some(vercel_misconfigured) = row.vercel_misconfigured {
            output.push_str(&format!("  vercel_misconfigured: {vercel_misconfigured}\n"));
        }
        if let Some(last_checked_at) = &row.last_checked_at {
            output.push_str("  last_checked_at: ");
            output.push_str(&yaml_scalar(last_checked_at));
            output.push('\n');
        }
        if let Some(last_error) = &row.last_error {
            output.push_str("  last_error: ");
            output.push_str(&yaml_scalar(last_error));
            output.push('\n');
        }
        if let Some(visits) = row.visits {
            output.push_str(&format!("  visits: {visits}\n"));
        }
    }
    output
}

fn yaml_scalar(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn install_manpage(requested_directory: Option<PathBuf>) -> Result<()> {
    let directories = requested_directory
        .map(|directory| vec![directory])
        .unwrap_or_else(default_manpage_directories);
    let mut failures = Vec::new();

    for directory in directories {
        match install_manpage_in(&directory) {
            Ok(destination) => {
                println!("Installed Suffix manual at {}", destination.display());
                return Ok(());
            }
            Err(error) => failures.push(format!("{}: {error:#}", directory.display())),
        }
    }

    bail!(
        "could not install the Suffix manual. Try `suffix man install --dir ~/.local/share/man/man1`, or run with the required system privileges.\n{}",
        failures.join("\n")
    )
}

fn install_manpage_in(directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(directory)
        .with_context(|| format!("could not create {}", directory.display()))?;
    let destination = directory.join("suffix.1");
    fs::write(&destination, MANPAGE)
        .with_context(|| format!("could not write {}", destination.display()))?;
    Ok(destination)
}

fn default_manpage_directories() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    manpage_directories_for(std::env::consts::OS, &home)
}

fn manpage_directories_for(platform: &str, home: &Path) -> Vec<PathBuf> {
    let user_directory = home.join(".local/share/man/man1");
    if platform == "macos" {
        vec![
            PathBuf::from("/opt/homebrew/share/man/man1"),
            PathBuf::from("/usr/local/share/man/man1"),
            user_directory,
        ]
    } else {
        vec![
            PathBuf::from("/usr/local/share/man/man1"),
            user_directory,
            PathBuf::from("/usr/share/man/man1"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn man_install_uses_standard_platform_directories_and_embeds_the_manual() {
        assert_eq!(
            manpage_directories_for("macos", Path::new("/Users/example")),
            vec![
                PathBuf::from("/opt/homebrew/share/man/man1"),
                PathBuf::from("/usr/local/share/man/man1"),
                PathBuf::from("/Users/example/.local/share/man/man1"),
            ]
        );
        assert_eq!(
            manpage_directories_for("linux", Path::new("/home/example")),
            vec![
                PathBuf::from("/usr/local/share/man/man1"),
                PathBuf::from("/home/example/.local/share/man/man1"),
                PathBuf::from("/usr/share/man/man1"),
            ]
        );
        assert!(MANPAGE.contains(".TH SUFFIX 1"));
    }

    #[test]
    fn man_install_writes_to_an_explicit_directory() {
        let directory =
            std::env::temp_dir().join(format!("suffix-man-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);

        let destination = install_manpage_in(&directory).unwrap();
        assert_eq!(fs::read_to_string(&destination).unwrap(), MANPAGE);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn login_url_carries_loopback_callback_and_state() {
        let url = build_login_url(
            "https://suffix.org",
            "http://127.0.0.1:44123/callback",
            "abc123abc123abc123",
            "suffix CLI on test",
            Some("person@example.com"),
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(
            parsed.as_str().split('?').next().unwrap(),
            "https://suffix.org/"
        );
        assert_eq!(params.get("cli_auth").map(|v| v.as_ref()), Some("1"));
        assert_eq!(
            params.get("callback").map(|v| v.as_ref()),
            Some("http://127.0.0.1:44123/callback")
        );
        assert_eq!(
            params.get("state").map(|v| v.as_ref()),
            Some("abc123abc123abc123")
        );
        assert_eq!(
            params.get("email").map(|v| v.as_ref()),
            Some("person@example.com")
        );
    }

    #[test]
    fn login_accepts_email_hint_positionally_or_by_flag() {
        let cli = Cli::try_parse_from(["suffix", "login", "Person@Example.COM"]).unwrap();
        match cli.command {
            Some(Command::Login(args)) => {
                assert_eq!(
                    login_email_hint(&args).unwrap().as_deref(),
                    Some("person@example.com")
                );
            }
            _ => panic!("expected login command"),
        }

        let cli =
            Cli::try_parse_from(["suffix", "login", "--email", "person@example.com"]).unwrap();
        match cli.command {
            Some(Command::Login(args)) => {
                assert_eq!(
                    login_email_hint(&args).unwrap().as_deref(),
                    Some("person@example.com")
                );
            }
            _ => panic!("expected login command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "login",
            "one@example.com",
            "--email",
            "two@example.com",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Login(args)) => {
                assert!(login_email_hint(&args).is_err());
            }
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn move_uses_the_active_account_when_one_other_account_is_given() {
        let mut config = Config {
            active_account: Some("personal@example.com".to_string()),
            ..Config::default()
        };
        for email in ["personal@example.com", "work@example.com"] {
            config.accounts.insert(
                email.to_string(),
                AccountConfig {
                    api_base: DEFAULT_API_BASE.to_string(),
                    api_key: Some(format!("key-{email}")),
                    email: Some(email.to_string()),
                    domain_count: None,
                    link_count: None,
                    visit_count: None,
                    last_checked_at: None,
                    last_error: None,
                },
            );
        }

        let to = MoveArgs {
            domain: "go.example.com".to_string(),
            account: Some("work@example.com".to_string()),
            target: None,
            to: false,
            from: false,
            yes: false,
        };
        assert_eq!(
            move_account_names(&config, &to).unwrap(),
            (
                "personal@example.com".to_string(),
                "work@example.com".to_string()
            )
        );

        let from = MoveArgs { from: true, ..to };
        assert_eq!(
            move_account_names(&config, &from).unwrap(),
            (
                "work@example.com".to_string(),
                "personal@example.com".to_string()
            )
        );
    }

    #[test]
    fn email_login_reuses_the_matching_saved_profile() {
        let mut config = Config::default();
        config.accounts.insert(
            "default".to_string(),
            AccountConfig {
                api_base: DEFAULT_API_BASE.to_string(),
                api_key: Some("curtail_sk_saved".to_string()),
                email: Some("person@example.com".to_string()),
                domain_count: None,
                link_count: None,
                visit_count: None,
                last_checked_at: None,
                last_error: None,
            },
        );

        let account = login_account_name(&config, None, Some("person@example.com")).unwrap();
        assert_eq!(account, "default");
        assert!(saved_key_matches(
            &config,
            &account,
            Some("person@example.com")
        ));
    }

    #[test]
    fn email_login_uses_a_distinct_profile_for_a_new_account() {
        let config = Config::default();
        let account = login_account_name(&config, None, Some("work@example.com")).unwrap();
        assert_eq!(account, "work@example.com");
        assert!(!saved_key_matches(
            &config,
            &account,
            Some("work@example.com")
        ));
    }

    #[test]
    fn saved_key_is_not_reused_for_a_different_email() {
        let mut config = Config::default();
        config.accounts.insert(
            "shared".to_string(),
            AccountConfig {
                api_base: DEFAULT_API_BASE.to_string(),
                api_key: Some("curtail_sk_saved".to_string()),
                email: Some("person@example.com".to_string()),
                domain_count: None,
                link_count: None,
                visit_count: None,
                last_checked_at: None,
                last_error: None,
            },
        );

        assert!(!saved_key_matches(
            &config,
            "shared",
            Some("other@example.com"),
        ));
    }

    #[test]
    fn callback_parser_reads_key_state_and_api_base() {
        let request = concat!(
            "GET /callback?state=s1&key=curtail_sk_abc&api_base=https%3A%2F%2Fsuffix.org%2Fapi%2Fv1 HTTP/1.1\r\n",
            "host: 127.0.0.1\r\n\r\n"
        );
        let callback = parse_callback(request).unwrap();
        assert_eq!(callback.state, "s1");
        assert_eq!(callback.key.as_deref(), Some("curtail_sk_abc"));
        assert_eq!(
            callback.api_base.as_deref(),
            Some("https://suffix.org/api/v1")
        );
        assert!(callback.error.is_none());
    }

    #[test]
    fn api_urls_target_the_static_v1_function() {
        let api = Api {
            client: Client::new(),
            base: "https://suffix.org/api/v1".to_string(),
            key: "secret".to_string(),
            account: "test".to_string(),
        };
        assert_eq!(
            api.url("shortcuts").unwrap(),
            "https://suffix.org/api/v1?resource=shortcuts"
        );
        assert_eq!(
            api.url("shortcuts?id=abc&version=3").unwrap(),
            "https://suffix.org/api/v1?resource=shortcuts&id=abc&version=3"
        );
    }

    #[test]
    fn terse_shortcut_commands_parse() {
        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "https://example.com/launch",
            "go",
            "--domain-id",
            "domain-1",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Add(args)) => {
                assert!(args.domain.is_none());
                assert_eq!(args.value, "https://example.com/launch");
                assert_eq!(args.tail.as_deref(), Some("go"));
                assert_eq!(args.domain_id.as_deref(), Some("domain-1"));
                assert_eq!(
                    shortcut_add_fields(&args).unwrap(),
                    ShortcutAddFields {
                        domain: ShortcutAddDomain::Id("domain-1".to_string()),
                        tail: "go".to_string(),
                        target_url: "https://example.com/launch".to_string(),
                    }
                );
            }
            _ => panic!("expected add command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "pair.rs/typesec",
            "https://github.com/querygraph/typesec",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Add(args)) => {
                assert_eq!(
                    shortcut_add_fields(&args).unwrap(),
                    ShortcutAddFields {
                        domain: ShortcutAddDomain::Hostname("pair.rs".to_string()),
                        tail: "typesec".to_string(),
                        target_url: "https://github.com/querygraph/typesec".to_string(),
                    }
                );
            }
            _ => panic!("expected add command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "pair.rs/typesec",
            "https://github.com/querygraph/typesec",
            "work@example.com",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Add(args)) => {
                assert_eq!(add_account(&args), Some("work@example.com"));
                assert_eq!(
                    shortcut_add_fields(&args).unwrap(),
                    ShortcutAddFields {
                        domain: ShortcutAddDomain::Hostname("pair.rs".to_string()),
                        tail: "typesec".to_string(),
                        target_url: "https://github.com/querygraph/typesec".to_string(),
                    }
                );
            }
            _ => panic!("expected add command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "-d",
            "pair.rs",
            "typesec",
            "https://github.com/querygraph/typesec",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Add(args)) => {
                assert_eq!(
                    shortcut_add_fields(&args).unwrap(),
                    ShortcutAddFields {
                        domain: ShortcutAddDomain::Hostname("pair.rs".to_string()),
                        tail: "typesec".to_string(),
                        target_url: "https://github.com/querygraph/typesec".to_string(),
                    }
                );
            }
            _ => panic!("expected add command"),
        }

        let cli = Cli::try_parse_from(["suffix", "domain", "ls", "work@example.com"]).unwrap();
        match cli.command {
            Some(Command::Domain(args)) => match args.command {
                DomainCliCommand::Ls(args) => {
                    assert_eq!(args.account.as_deref(), Some("work@example.com"));
                }
                _ => panic!("expected domain list"),
            },
            _ => panic!("expected domain command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "rm",
            "shortcut-1",
            "work@example.com",
            "--version",
            "7",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Rm(args)) => {
                assert!(!args.domain);
                assert_eq!(args.id, "shortcut-1");
                assert_eq!(args.account.as_deref(), Some("work@example.com"));
                assert_eq!(args.version, Some(7));
            }
            _ => panic!("expected rm command"),
        }
    }

    #[test]
    fn public_shortcut_commands_support_generated_and_custom_tails() {
        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "--public",
            "--letters",
            "--words",
            "https://example.com/launch",
        ])
        .unwrap();
        let Some(Command::Add(args)) = cli.command else {
            panic!("expected add command")
        };
        assert!(args.public && args.letters && args.words);
        assert_eq!(
            shortcut_add_fields(&args).unwrap(),
            ShortcutAddFields {
                domain: ShortcutAddDomain::Hostname("suf.cx".to_string()),
                tail: String::new(),
                target_url: "https://example.com/launch".to_string(),
            }
        );
        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "--public",
            "dog-cat",
            "https://example.com/launch",
        ])
        .unwrap();
        let Some(Command::Add(args)) = cli.command else {
            panic!("expected add command")
        };
        assert_eq!(
            shortcut_add_fields(&args).unwrap(),
            ShortcutAddFields {
                domain: ShortcutAddDomain::Hostname("suf.cx".to_string()),
                tail: "dog-cat".to_string(),
                target_url: "https://example.com/launch".to_string(),
            }
        );
    }

    #[test]
    fn photo_commands_parse_and_build_api_fields() {
        let cli = Cli::try_parse_from([
            "suffix",
            "add",
            "foto.gs/sunset",
            "https://example.com/sunset.jpg",
            "--photo",
            "--photo-mode",
            "local",
            "--protect",
            "--password-file",
            "./secret.txt",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Add(args)) => {
                assert!(args.photo);
                assert_eq!(args.photo_mode, PhotoMode::Local);
                assert!(args.protect);
                assert_eq!(args.password_file, Some(PathBuf::from("./secret.txt")));
                let payload = shortcut_payload(json!({}), args.photo, args.photo_mode);
                assert_eq!(payload, json!({ "type": "photo", "photoMode": "local" }));
            }
            _ => panic!("expected add command"),
        }

        let cli =
            Cli::try_parse_from(["suffix", "photo", "shortcut-1", "-r", "--version", "4"]).unwrap();
        match cli.command {
            Some(Command::Photo(args)) => {
                assert!(!args.local);
                assert!(args.remote);
                assert_eq!(args.version, Some(4));
            }
            _ => panic!("expected photo command"),
        }

        assert!(Cli::try_parse_from(["suffix", "photo", "shortcut-1", "-l", "-r"]).is_err());

        let cli = Cli::try_parse_from(["suffix", "photo", "shortcut-1", "--drop"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Photo(PhotoArgs { drop: true, .. }))
        ));

        let cli = Cli::try_parse_from([
            "suffix",
            "password",
            "shortcut-1",
            "--add",
            "--password-file",
            "./secret.txt",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Password(PasswordArgs { add: true, .. }))
        ));

        let cli = Cli::try_parse_from(["suffix", "password", "shortcut-1", "--remove"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Password(PasswordArgs { remove: true, .. }))
        ));

        let cli =
            Cli::try_parse_from(["suffix", "edit", "shortcut-1", "--tail", "new-tail"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Edit(EditArgs { tail: Some(_), .. }))
        ));
    }

    #[test]
    fn domain_namespace_and_account_commands_parse() {
        let cli = Cli::try_parse_from(["suffix"]).unwrap();
        assert!(cli.command.is_none());

        let cli = Cli::try_parse_from(["suffix", "ls", "-l", "work@example.com"]).unwrap();
        match cli.command {
            Some(Command::Ls(args)) => {
                assert!(args.stats);
                assert_eq!(args.account.as_deref(), Some("work@example.com"));
            }
            _ => panic!("expected shortcut list"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "domain",
            "add",
            "go.example.com",
            "work@example.com",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Domain(args)) => match args.command {
                DomainCliCommand::Add { hostname, account } => {
                    assert_eq!(hostname, "go.example.com");
                    assert_eq!(account.as_deref(), Some("work@example.com"));
                }
                _ => panic!("expected domain add"),
            },
            _ => panic!("expected domain command"),
        }

        let cli = Cli::try_parse_from(["suffix", "account", "work"]).unwrap();
        match cli.command {
            Some(Command::Account(args)) => assert_eq!(args.args, ["work"]),
            _ => panic!("expected account command"),
        }

        let cli = Cli::try_parse_from(["suffix", "account", "ls", "-l"]).unwrap();
        match cli.command {
            Some(Command::Account(args)) => {
                assert_eq!(args.args, ["ls"]);
                assert!(args.long);
            }
            _ => panic!("expected account command"),
        }

        let cli = Cli::try_parse_from(["suffix", "logout", "person@example.com"]).unwrap();
        match cli.command {
            Some(Command::Logout { account }) => assert_eq!(account, "person@example.com"),
            _ => panic!("expected logout command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "mv",
            "pair.rs",
            "owner@example.com",
            "target@example.com",
            "--yes",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Mv(args)) => {
                assert_eq!(args.domain, "pair.rs");
                assert_eq!(args.account.as_deref(), Some("owner@example.com"));
                assert_eq!(args.target.as_deref(), Some("target@example.com"));
                assert!(args.yes);
            }
            _ => panic!("expected move command"),
        }

        let cli =
            Cli::try_parse_from(["suffix", "mv", "--to", "pair.rs", "target@example.com"]).unwrap();
        match cli.command {
            Some(Command::Mv(args)) => {
                assert_eq!(args.account.as_deref(), Some("target@example.com"));
                assert!(args.to);
                assert!(!args.from);
            }
            _ => panic!("expected move command"),
        }

        let cli = Cli::try_parse_from(["suffix", "mv", "--from", "pair.rs", "owner@example.com"])
            .unwrap();
        match cli.command {
            Some(Command::Mv(args)) => {
                assert_eq!(args.account.as_deref(), Some("owner@example.com"));
                assert!(args.from);
                assert!(!args.to);
            }
            _ => panic!("expected move command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "transfer",
            "pair.rs",
            "--to",
            "target@example.com",
            "--minutes",
            "30",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Transfer(args)) => {
                assert_eq!(args.domain, "pair.rs");
                assert_eq!(args.to.as_deref(), Some("target@example.com"));
                assert_eq!(args.minutes, 30);
            }
            _ => panic!("expected transfer command"),
        }

        let cli = Cli::try_parse_from(["suffix", "accept", "SUF-X7K9-Q2M"]).unwrap();
        match cli.command {
            Some(Command::Accept(args)) => assert_eq!(args.code, "SUF-X7K9-Q2M"),
            _ => panic!("expected accept command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "account",
            "add",
            "work",
            "--key",
            "curtail_sk_test",
            "--email",
            "work@example.com",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Account(args)) => {
                assert_eq!(args.args, ["add", "work"]);
                assert_eq!(args.key.as_deref(), Some("curtail_sk_test"));
                assert_eq!(args.email.as_deref(), Some("work@example.com"));
            }
            _ => panic!("expected account command"),
        }

        let cli = Cli::try_parse_from(["suffix", "domain", "ls", "-l", "--json"]).unwrap();
        match cli.command {
            Some(Command::Domain(args)) => match args.command {
                DomainCliCommand::Ls(args) => {
                    assert!(args.stats);
                    assert!(args.json);
                }
                _ => panic!("expected domain list"),
            },
            _ => panic!("expected domain command"),
        }
    }

    #[test]
    fn legacy_default_profiles_are_discarded() {
        let mut config = Config {
            api_base: Some("https://suffix.org/api/v1".to_string()),
            api_key: Some("curtail_sk_old".to_string()),
            active_account: Some("default".to_string()),
            accounts: BTreeMap::new(),
        };
        config.accounts.insert(
            "default".to_string(),
            AccountConfig {
                api_base: DEFAULT_API_BASE.to_string(),
                api_key: Some("curtail_sk_old".to_string()),
                email: None,
                domain_count: None,
                link_count: None,
                visit_count: None,
                last_checked_at: None,
                last_error: None,
            },
        );

        assert!(discard_legacy_profiles(&mut config));
        assert!(config.active_account.is_none());
        assert!(config.api_base.is_none());
        assert!(config.api_key.is_none());
        assert!(config.accounts.is_empty());
    }

    #[test]
    fn account_selector_matches_name_or_cached_email() {
        let mut config = Config::default();
        config.accounts.insert(
            "personal".to_string(),
            AccountConfig {
                api_base: DEFAULT_API_BASE.to_string(),
                api_key: Some("key".to_string()),
                email: Some("person@example.com".to_string()),
                domain_count: Some(2),
                link_count: Some(5),
                visit_count: Some(13),
                last_checked_at: Some("123".to_string()),
                last_error: None,
            },
        );
        assert_eq!(account_selector(&config, "personal").unwrap(), "personal");
        assert_eq!(
            account_selector(&config, "Person@Example.COM").unwrap(),
            "personal"
        );
        assert!(account_selector(&config, "missing@example.com").is_err());
    }

    #[test]
    fn tail_derives_from_url_path() {
        assert_eq!(derive_tail("https://example.com/path/launch"), "launch");
        assert_eq!(derive_tail("https://example.com/"), "link");
    }

    #[test]
    fn shortcut_rows_normalize_api_payload() {
        let payload = json!({
            "shortcuts": [{
                "hostname": "go.example.com",
                "tail": "launch",
                "targetUrl": "https://example.com/launch",
                "clickCount": 42
            }]
        });
        let rows = shortcut_rows(&payload, true).unwrap();
        assert_eq!(rows[0].shortcut, "go.example.com/launch");
        assert_eq!(rows[0].target, "https://example.com/launch");
        assert_eq!(rows[0].visits, Some(42));

        let rows = shortcut_rows(&payload, false).unwrap();
        assert_eq!(rows[0].visits, None);
    }

    #[test]
    fn domain_rows_aggregate_shortcut_visits() {
        let domains = json!({
            "domains": [
                {
                    "id": "domain-1",
                    "hostname": "go.example.com",
                    "status": "verified",
                    "ownerName": "Test User",
                    "ownerEmail": "test@example.com",
                    "vercelVerified": true
                },
                {
                    "id": "domain-2",
                    "hostname": "jump.example.com",
                    "status": "pending"
                }
            ]
        });
        let shortcuts = json!({
            "shortcuts": [
                { "domainId": "domain-1", "clickCount": 3 },
                { "domainId": "domain-1", "clickCount": 4 },
                { "domainId": "domain-2", "clickCount": 0 }
            ]
        });
        let visits = domain_visit_counts(&shortcuts).unwrap();
        let rows = domain_rows(&domains, Some(&visits)).unwrap();
        assert_eq!(rows[0].hostname, "go.example.com");
        assert_eq!(rows[0].status, "verified");
        assert_eq!(rows[0].owner_name.as_deref(), Some("Test User"));
        assert_eq!(rows[0].visits, Some(7));
        assert_eq!(rows[1].visits, Some(0));

        let rows = domain_rows(&domains, None).unwrap();
        assert_eq!(rows[0].visits, None);
    }

    #[test]
    fn list_output_format_rejects_multiple_structured_flags() {
        assert!(output_format(true, true, false).is_err());
    }

    #[test]
    fn xml_output_escapes_shortcuts() {
        let rows = [ShortcutRow {
            shortcut: "go.example.com/a&b".to_string(),
            target: "https://example.com/?a=1&b=2".to_string(),
            visits: Some(3),
        }];
        let xml = shortcuts_xml(&rows);
        assert!(xml.contains("<short>go.example.com/a&amp;b</short>"));
        assert!(xml.contains("<target>https://example.com/?a=1&amp;b=2</target>"));
        assert!(xml.contains("<visits>3</visits>"));
    }

    #[test]
    fn yaml_output_quotes_shortcuts() {
        let rows = [ShortcutRow {
            shortcut: "go.example.com/a".to_string(),
            target: "https://example.com/\"quoted\"".to_string(),
            visits: None,
        }];
        let yaml = shortcuts_yaml(&rows);
        assert!(yaml.contains("- shortcut: \"go.example.com/a\""));
        assert!(yaml.contains("target: \"https://example.com/\\\"quoted\\\"\""));
    }

    #[test]
    fn domain_structured_output_includes_verbose_fields() {
        let rows = [DomainRow {
            id: "domain-1".to_string(),
            hostname: "go.example.com".to_string(),
            status: "needs<dns>".to_string(),
            owner_name: Some("Test User".to_string()),
            owner_email: Some("test@example.com".to_string()),
            vercel_verified: Some(false),
            vercel_misconfigured: Some(true),
            last_checked_at: Some("2026-07-16T00:00:00Z".to_string()),
            last_error: Some("A & CNAME conflict".to_string()),
            visits: Some(11),
        }];
        let xml = domains_xml(&rows);
        assert!(xml.contains("<hostname>go.example.com</hostname>"));
        assert!(xml.contains("<status>needs&lt;dns&gt;</status>"));
        assert!(xml.contains("<last_error>A &amp; CNAME conflict</last_error>"));
        assert!(xml.contains("<visits>11</visits>"));

        let yaml = domains_yaml(&rows);
        assert!(yaml.contains("- id: \"domain-1\""));
        assert!(yaml.contains("owner_email: \"test@example.com\""));
        assert!(yaml.contains("vercel_misconfigured: true"));
    }

    #[test]
    fn upload_command_parses_and_sanitizes_file_names() {
        let cli = Cli::try_parse_from([
            "suffix",
            "upload",
            "files.example/report",
            "./Board Notes.pdf",
            "--title",
            "Board report",
            "--protect",
            "--password-file",
            "./secret.txt",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Upload(args)) => {
                assert_eq!(args.value, "files.example/report");
                assert_eq!(args.file, PathBuf::from("./Board Notes.pdf"));
                assert!(args.protect);
                let (domain, tail) = upload_locator(&args).unwrap();
                assert!(
                    matches!(domain, ShortcutAddDomain::Hostname(value) if value == "files.example")
                );
                assert_eq!(tail, "report");
            }
            _ => panic!("expected upload command"),
        }
        assert_eq!(safe_upload_name("../secret\0.txt"), ".._secret_.txt");
        assert_eq!(file_content_type("report.pdf"), "application/pdf");
        assert_eq!(random_uuid().len(), 36);
    }

    #[test]
    fn search_and_creation_conflict_helpers_cover_both_fields() {
        let cli = Cli::try_parse_from(["suffix", "search", "foto\\.gs|report.*pdf"]).unwrap();
        assert!(
            matches!(cli.command, Some(Command::Search(SearchArgs { pattern, .. })) if pattern == "foto\\.gs|report.*pdf")
        );

        let shortcuts = vec![json!({
            "id": "link-1", "version": 2, "domainId": "domain-1",
            "hostname": "go.example", "tail": "report", "targetUrl": "https://files.example/report.pdf"
        })];
        let occupied = occupied_tail(
            &shortcuts,
            &json!({ "domainId": "domain-1", "tail": "report" }),
        )
        .unwrap();
        assert_eq!(shortcut_name(occupied), "go.example/report");
        assert!(same_target(
            "https://files.example",
            "https://files.example/"
        ));

        let add = Cli::try_parse_from([
            "suffix",
            "add",
            "go.example/new",
            "https://files.example/report.pdf",
            "--allow-duplicate-target",
        ])
        .unwrap();
        assert!(matches!(
            add.command,
            Some(Command::Add(AddArgs {
                allow_duplicate_target: true,
                ..
            }))
        ));
    }
}
