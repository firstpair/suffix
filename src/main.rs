use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    net::TcpListener,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use rand::{RngExt, distr::Alphanumeric};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

const DEFAULT_APP_URL: &str = "https://suffix.org";
const DEFAULT_API_BASE: &str = "https://suffix.org/api/v1";

#[derive(Parser)]
#[command(name = "suffix", version, about = "CLI client for suffix.org")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Open the browser, approve a one-time API key, and save it locally.
    Login(LoginArgs),
    /// Forget a saved account's API key but keep cached account metadata.
    Logout { account: String },
    /// List shortcuts.
    Ls(ListArgs),
    /// Add a shortcut.
    Add(AddArgs),
    /// Remove a shortcut.
    Rm(RemoveArgs),
    /// Move a domain from one logged-in account to another.
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
    /// Manage shortcuts. Prefer `suffix ls`, `suffix add`, and `suffix rm`.
    #[command(hide = true)]
    Shortcuts(ShortcutsArgs),
    /// Manage domains. Prefer `suffix ls --domains`, `suffix add --domain`, and `suffix rm --domain`.
    #[command(hide = true)]
    Domains(DomainsArgs),
}

#[derive(Args)]
struct LoginArgs {
    /// Email address to hint in the browser sign-in flow.
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
    /// Print the login URL instead of opening the browser.
    #[arg(long)]
    no_open: bool,
    /// Seconds to wait for browser approval.
    #[arg(long, default_value_t = 120)]
    timeout: u64,
}

#[derive(Args)]
struct ListArgs {
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
struct DomainListArgs {
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
    /// Target URL, or `HOST/TAIL` when the target URL is passed next.
    value: String,
    /// Shortcut tail, or target URL when VALUE is `HOST/TAIL`.
    tail: Option<String>,
    /// Target URL when using `-d HOST TAIL URL`.
    target_url: Option<String>,
    /// Domain ID for a new shortcut. Defaults to the first owned domain.
    #[arg(long)]
    domain_id: Option<String>,
    /// Optional shortcut title.
    #[arg(long)]
    title: Option<String>,
}

#[derive(Args)]
struct RemoveArgs {
    /// Remove a domain instead of a shortcut. Prefer `suffix domain rm ID`.
    #[arg(long, hide = true)]
    domain: bool,
    /// Shortcut or domain ID.
    id: String,
    /// Shortcut version. If omitted, suffix looks it up first.
    #[arg(long)]
    version: Option<u64>,
}

#[derive(Args)]
struct MoveArgs {
    /// Domain hostname or ID to move.
    domain: String,
    /// Source stored account name or email.
    from: String,
    /// Target stored account name or email.
    to: String,
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
    Add { hostname: String },
    /// Remove an empty domain.
    Rm { id: String },
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
            let api = Api::from_config()?;
            if args.domains {
                api.get("domains")
            } else {
                list_shortcuts(&api, args)
            }
        }
        Some(Command::Add(args)) => add(args),
        Some(Command::Rm(args)) => remove(args),
        Some(Command::Mv(args)) => move_domain(args),
        Some(Command::Transfer(args)) => transfer_code(args),
        Some(Command::Accept(args)) => accept_transfer(args),
        Some(Command::Domain(args)) => {
            let api = Api::from_config()?;
            match args.command {
                DomainCliCommand::Ls(args) => list_domains(&api, args),
                DomainCliCommand::Add { hostname } => {
                    api.post("domains", json!({ "hostname": hostname }))
                }
                DomainCliCommand::Rm { id } => api.delete(&format!("domains?id={}", encode(&id))),
            }
        }
        Some(Command::Account(args)) => account(args),
        Some(Command::Config) => print_config(),
        Some(Command::Shortcuts(args)) => {
            let api = Api::from_config()?;
            match args.command {
                ShortcutCommand::List => api.get("shortcuts"),
                ShortcutCommand::Create { domain_id, tail, target_url, title } => api.post(
                    "shortcuts",
                    json!({ "domainId": domain_id, "tail": tail, "targetUrl": target_url, "title": title }),
                ),
                ShortcutCommand::Update { id, version, domain_id, tail, target_url, title, active } => api.patch(
                    "shortcuts",
                    json!({
                        "id": id,
                        "version": version,
                        "domainId": domain_id,
                        "tail": tail,
                        "targetUrl": target_url,
                        "title": title,
                        "isActive": active,
                    }),
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
            let api = Api::from_config()?;
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
    let listener =
        TcpListener::bind("127.0.0.1:0").context("could not bind the local login callback")?;
    listener
        .set_nonblocking(false)
        .context("could not configure the local login callback")?;
    let port = listener.local_addr()?.port();
    let state = random_state();
    let callback = format!("http://127.0.0.1:{port}/callback");
    let account = normalize_account_name(args.account.as_deref().unwrap_or("default"))?;
    let login_email = login_email_hint(&args)?;
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

    let mut config = load_config()?;
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
    let api = Api::from_config()?;
    let shortcut = shortcut_add_fields(&args)?;
    let mut payload = json!({
        "tail": shortcut.tail,
        "targetUrl": shortcut.target_url,
        "title": args.title,
    });
    match shortcut.domain {
        ShortcutAddDomain::Hostname(hostname) => payload["domain"] = json!(hostname),
        ShortcutAddDomain::Id(domain_id) => payload["domainId"] = json!(domain_id),
        ShortcutAddDomain::Default => payload["domainId"] = json!(api.default_domain_id()?),
    }
    api.post("shortcuts", payload)
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
    if let Some(hostname) = args.domain.as_deref() {
        let target_url = args
            .target_url
            .as_deref()
            .or(args.tail.as_deref())
            .ok_or_else(|| anyhow!("usage: suffix add -d HOST TAIL URL"))?;
        return Ok(ShortcutAddFields {
            domain: ShortcutAddDomain::Hostname(hostname.to_string()),
            tail: args.value.clone(),
            target_url: target_url.to_string(),
        });
    }
    if args.target_url.is_some() {
        bail!("usage: suffix add -d HOST TAIL URL or suffix add HOST/TAIL URL");
    }
    if let Some(target_url) = args.tail.as_deref() {
        if !looks_like_url(&args.value) && looks_like_url(target_url) {
            let (hostname, tail) = split_shortcut_locator(&args.value)?;
            return Ok(ShortcutAddFields {
                domain: ShortcutAddDomain::Hostname(hostname),
                tail,
                target_url: target_url.to_string(),
            });
        }
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
    let api = Api::from_config()?;
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
    let from_name = account_selector(&config, &args.from)?;
    let to_name = account_selector(&config, &args.to)?;
    if from_name == to_name {
        bail!("source and target accounts are the same");
    }
    let from = config
        .accounts
        .get(&from_name)
        .ok_or_else(|| anyhow!("no stored account matching {}", args.from))?;
    let to = config
        .accounts
        .get(&to_name)
        .ok_or_else(|| anyhow!("no stored account matching {}", args.to))?;
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
        let response = request
            .bearer_auth(&self.key)
            .send()
            .context("request failed")?;
        let status = response.status();
        let payload: Value = response.json().unwrap_or_else(|_| json!({}));
        if !status.is_success() {
            let code = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("request_failed");
            let message = payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("suffix.org rejected the request");
            bail!("{code}: {message}");
        }
        Ok(payload)
    }

    fn default_domain_id(&self) -> Result<String> {
        let payload = self.request_json(self.client.get(self.url("domains")?))?;
        let domains = payload
            .get("domains")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("suffix.org did not return a domain list"))?;
        if domains.is_empty() {
            bail!(
                "no domains found for {}; add one with `suffix add --domain HOST` or pass --domain-id",
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
        _ => bail!("unsupported API resource {name}"),
    }
}

fn load_config() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let contents =
        fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
    let mut config: Config =
        toml::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))?;
    migrate_single_account_config(&mut config);
    Ok(config)
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    fs::write(&path, toml::to_string(config)?)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
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
    let dir =
        dirs::config_dir().ok_or_else(|| anyhow!("could not find the user config directory"))?;
    Ok(dir.join("suffix").join("config.toml"))
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

fn first_logged_in_account(config: &Config) -> Option<String> {
    config
        .accounts
        .iter()
        .find(|(_, account)| account.api_key.is_some())
        .map(|(name, _)| name.clone())
}

fn migrate_single_account_config(config: &mut Config) {
    if config.accounts.is_empty()
        && let (Some(api_base), Some(api_key)) = (config.api_base.take(), config.api_key.take())
    {
        config.accounts.insert(
            "default".to_string(),
            AccountConfig {
                api_base,
                api_key: Some(api_key),
                email: None,
                domain_count: None,
                link_count: None,
                visit_count: None,
                last_checked_at: None,
                last_error: None,
            },
        );
        config.active_account = Some("default".to_string());
    }
}

fn print_accounts(config: &Config, long: bool) {
    if config.accounts.is_empty() {
        println!("No accounts. Run `suffix login --account NAME`.");
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
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("account names may use letters, numbers, dash, underscore, or dot");
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

#[cfg(test)]
mod tests {
    use super::*;

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

        let cli = Cli::try_parse_from(["suffix", "rm", "shortcut-1", "--version", "7"]).unwrap();
        match cli.command {
            Some(Command::Rm(args)) => {
                assert!(!args.domain);
                assert_eq!(args.id, "shortcut-1");
                assert_eq!(args.version, Some(7));
            }
            _ => panic!("expected rm command"),
        }
    }

    #[test]
    fn domain_namespace_and_account_commands_parse() {
        let cli = Cli::try_parse_from(["suffix"]).unwrap();
        assert!(cli.command.is_none());

        let cli = Cli::try_parse_from(["suffix", "ls", "-l"]).unwrap();
        match cli.command {
            Some(Command::Ls(args)) => assert!(args.stats),
            _ => panic!("expected shortcut list"),
        }

        let cli = Cli::try_parse_from(["suffix", "domain", "add", "go.example.com"]).unwrap();
        match cli.command {
            Some(Command::Domain(args)) => match args.command {
                DomainCliCommand::Add { hostname } => assert_eq!(hostname, "go.example.com"),
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
                assert_eq!(args.from, "owner@example.com");
                assert_eq!(args.to, "target@example.com");
                assert!(args.yes);
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
    fn single_account_config_migrates_to_named_accounts() {
        let mut config = Config {
            api_base: Some("https://suffix.org/api/v1".to_string()),
            api_key: Some("curtail_sk_old".to_string()),
            active_account: None,
            accounts: BTreeMap::new(),
        };
        migrate_single_account_config(&mut config);
        assert_eq!(config.active_account.as_deref(), Some("default"));
        assert!(config.api_base.is_none());
        assert!(config.api_key.is_none());
        assert_eq!(
            config
                .accounts
                .get("default")
                .and_then(|account| account.api_key.as_deref()),
            Some("curtail_sk_old")
        );
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
}
