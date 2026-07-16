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
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open the browser, approve a one-time API key, and save it locally.
    Login(LoginArgs),
    /// List shortcuts.
    Ls(ListArgs),
    /// Add a shortcut.
    Add(AddArgs),
    /// Remove a shortcut.
    Rm(RemoveArgs),
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
    /// Curtail dashboard URL to open.
    #[arg(long, default_value = DEFAULT_APP_URL)]
    app_url: String,
    /// Local profile name to save this key under.
    #[arg(long)]
    account: Option<String>,
    /// API key name shown in the Curtail dashboard.
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
    #[arg(long)]
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
    #[arg(long)]
    stats: bool,
}

#[derive(Args)]
struct AddArgs {
    /// Add a domain instead of a shortcut. Prefer `suffix domain add HOST`.
    #[arg(long, hide = true)]
    domain: bool,
    /// Target URL for a shortcut, or hostname when --domain is set.
    value: String,
    /// Shortcut tail. If omitted, suffix derives one from the URL path.
    tail: Option<String>,
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
    api_key: String,
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
        Command::Login(args) => login(args),
        Command::Ls(args) => {
            let api = Api::from_config()?;
            if args.domains {
                api.get("domains")
            } else {
                list_shortcuts(&api, args)
            }
        }
        Command::Add(args) => add(args),
        Command::Rm(args) => remove(args),
        Command::Domain(args) => {
            let api = Api::from_config()?;
            match args.command {
                DomainCliCommand::Ls(args) => list_domains(&api, args),
                DomainCliCommand::Add { hostname } => {
                    api.post("domains", json!({ "hostname": hostname }))
                }
                DomainCliCommand::Rm { id } => api.delete(&format!("domains?id={}", encode(&id))),
            }
        }
        Command::Account(args) => account(args),
        Command::Config => print_config(),
        Command::Shortcuts(args) => {
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
        Command::Domains(args) => {
            let api = Api::from_config()?;
            match args.command {
                DomainCommand::List => api.get("domains"),
                DomainCommand::Add { hostname } => {
                    api.post("domains", json!({ "hostname": hostname }))
                }
                DomainCommand::Delete { id } => api.delete(&format!("domains?id={}", encode(&id))),
            }
        }
        Command::Stats(args) => {
            let api = Api::from_config()?;
            api.get(&format!(
                "stats?shortcutId={}&days={}",
                encode(&args.shortcut_id),
                args.days
            ))
        }
    }
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
    let key_name = args.name.unwrap_or_else(|| default_key_name(&account));
    let login_url = build_login_url(&args.app_url, &callback, &state, &key_name)?;

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
            api_key,
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
    if args.domain {
        return api.post("domains", json!({ "hostname": args.value }));
    }
    let domain_id = match args.domain_id {
        Some(value) => value,
        None => api.default_domain_id()?,
    };
    let tail = args.tail.unwrap_or_else(|| derive_tail(&args.value));
    api.post(
        "shortcuts",
        json!({
            "domainId": domain_id,
            "tail": tail,
            "targetUrl": args.value,
            "title": args.title,
        }),
    )
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

fn account(args: AccountArgs) -> Result<()> {
    match args.args.as_slice() {
        [] => {
            let config = load_config()?;
            print_accounts(&config);
        }
        [name] => {
            let mut config = load_config()?;
            let name = normalize_account_name(name)?;
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
                    api_key: key,
                },
            );
            config.active_account = Some(name.clone());
            save_config(&config)?;
            println!("Switched to {name}");
        }
        [action, name] if action == "rm" => {
            let name = normalize_account_name(name)?;
            let mut config = load_config()?;
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
                "usage: suffix account | suffix account NAME | suffix account add NAME --key KEY | suffix account rm NAME"
            )
        }
    }
    Ok(())
}

fn build_login_url(app_url: &str, callback: &str, state: &str, name: &str) -> Result<String> {
    let mut url = Url::parse(app_url).context("app URL must be absolute")?;
    url.query_pairs_mut()
        .append_pair("cli_auth", "1")
        .append_pair("callback", callback)
        .append_pair("state", state)
        .append_pair("name", name);
    Ok(url.to_string())
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
            .or_else(|| active_account(&config).map(|(_, account)| account.api_key.clone()))
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
        || active_account(&config).is_some()
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

fn migrate_single_account_config(config: &mut Config) {
    if config.accounts.is_empty()
        && let (Some(api_base), Some(api_key)) = (config.api_base.take(), config.api_key.take())
    {
        config
            .accounts
            .insert("default".to_string(), AccountConfig { api_base, api_key });
        config.active_account = Some("default".to_string());
    }
}

fn print_accounts(config: &Config) {
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
        println!("{marker} {name}\t{}", account.api_base);
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
            Command::Add(args) => {
                assert!(!args.domain);
                assert_eq!(args.value, "https://example.com/launch");
                assert_eq!(args.tail.as_deref(), Some("go"));
                assert_eq!(args.domain_id.as_deref(), Some("domain-1"));
            }
            _ => panic!("expected add command"),
        }

        let cli = Cli::try_parse_from(["suffix", "rm", "shortcut-1", "--version", "7"]).unwrap();
        match cli.command {
            Command::Rm(args) => {
                assert!(!args.domain);
                assert_eq!(args.id, "shortcut-1");
                assert_eq!(args.version, Some(7));
            }
            _ => panic!("expected rm command"),
        }
    }

    #[test]
    fn domain_namespace_and_account_commands_parse() {
        let cli = Cli::try_parse_from(["suffix", "domain", "add", "go.example.com"]).unwrap();
        match cli.command {
            Command::Domain(args) => match args.command {
                DomainCliCommand::Add { hostname } => assert_eq!(hostname, "go.example.com"),
                _ => panic!("expected domain add"),
            },
            _ => panic!("expected domain command"),
        }

        let cli = Cli::try_parse_from(["suffix", "account", "work"]).unwrap();
        match cli.command {
            Command::Account(args) => assert_eq!(args.args, ["work"]),
            _ => panic!("expected account command"),
        }

        let cli = Cli::try_parse_from([
            "suffix",
            "account",
            "add",
            "work",
            "--key",
            "curtail_sk_test",
        ])
        .unwrap();
        match cli.command {
            Command::Account(args) => {
                assert_eq!(args.args, ["add", "work"]);
                assert_eq!(args.key.as_deref(), Some("curtail_sk_test"));
            }
            _ => panic!("expected account command"),
        }

        let cli = Cli::try_parse_from(["suffix", "domain", "ls", "--stats", "--json"]).unwrap();
        match cli.command {
            Command::Domain(args) => match args.command {
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
                .map(|account| account.api_key.as_str()),
            Some("curtail_sk_old")
        );
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
