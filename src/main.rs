use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::Duration,
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use rand::{distributions::Alphanumeric, Rng};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    /// Show the active CLI configuration without printing the secret.
    Config,
    /// Manage shortcuts.
    Shortcuts(ShortcutsArgs),
    /// Manage domains.
    Domains(DomainsArgs),
    /// Read aggregate shortcut statistics.
    Stats(StatsArgs),
}

#[derive(Args)]
struct LoginArgs {
    /// Curtail dashboard URL to open.
    #[arg(long, default_value = DEFAULT_APP_URL)]
    app_url: String,
    /// API key name shown in the Curtail dashboard.
    #[arg(long)]
    name: Option<String>,
    /// Print the login URL instead of opening the browser.
    #[arg(long)]
    no_open: bool,
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
    api_base: Option<String>,
    api_key: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Login(args) => login(args),
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
    let key_name = args.name.unwrap_or_else(default_key_name);
    let login_url = build_login_url(&args.app_url, &callback, &state, &key_name)?;

    println!("Opening {login_url}");
    if args.no_open {
        println!("Paste that URL into your browser to continue.");
    } else {
        open::that(&login_url).context("could not open the browser")?;
    }
    println!("Waiting for browser approval...");

    listener
        .set_nonblocking(false)
        .context("could not configure the local login callback")?;
    let (mut stream, _) = listener
        .accept()
        .context("could not accept the login callback")?;
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

    save_config(&Config {
        api_base: Some(api_base.clone()),
        api_key: Some(api_key),
    })?;
    respond(
        &mut stream,
        200,
        "suffix login complete. You can close this tab.",
    )?;
    println!("Saved suffix.org credentials for {api_base}");
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
}

impl Api {
    fn from_config() -> Result<Self> {
        let config = load_config()?;
        let base = std::env::var("SUFFIX_API_BASE")
            .ok()
            .or(config.api_base)
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        let key = std::env::var("SUFFIX_API_KEY")
            .ok()
            .or(config.api_key)
            .ok_or_else(|| anyhow!("not logged in; run `suffix login` or set SUFFIX_API_KEY"))?;
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .context("could not build HTTP client")?,
            base: base.trim_end_matches('/').to_string(),
            key,
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
        println!("{}", serde_json::to_string_pretty(&payload)?);
        Ok(())
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
    toml::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
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
        .or(config.api_base)
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let has_key = std::env::var("SUFFIX_API_KEY").is_ok() || config.api_key.is_some();
    println!("api_base = {base}");
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
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn default_key_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "local machine".to_string());
    format!("suffix CLI on {host}")
}

fn encode(value: &str) -> String {
    urlencoding::encode(value).into_owned()
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
}
