//! `hq github`: connect this HQ to the person's own GitHub App, see where it stands, disconnect.

use std::net::Ipv4Addr;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Subcommand;
use hq_tools::github::app::{AppClient, AppRecord, Standing, TokenRefused, standing};
use hq_web::github_connect_api::{BrowserFlow, browser_routes};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Subcommand)]
pub enum GithubAction {
    /// Create your own GitHub App in the browser and install it on the repositories you pick
    Connect,
    /// Whether GitHub is connected, and on which account
    Status,
    /// Delete the App's key from this device and show where to delete the App on GitHub
    Disconnect,
    /// Let HQ work on a repository that was flagged for using secrets or preview builds
    Confirm { repo: String },
    /// Agree to the first queued task after connecting
    FirstTask,
}

pub async fn run(action: GithubAction) -> Result<()> {
    let client = AppClient::standard()?;
    match action {
        GithubAction::Connect => connect(client).await,
        GithubAction::Status => status(client).await,
        GithubAction::Disconnect => disconnect(&client),
        GithubAction::Confirm { repo } => confirm(client, &repo).await,
        GithubAction::FirstTask => first_task(&client),
    }
}

async fn confirm(client: AppClient, repo: &str) -> Result<()> {
    client.scan_repositories().await?;
    match client.confirm_repository(repo)? {
        true => println!("HQ may now work on {repo}."),
        false => println!("HQ's App does not reach {repo}."),
    }
    Ok(())
}

fn first_task(client: &AppClient) -> Result<()> {
    match client.confirm_first_task()? {
        true => println!("HQ may start its first queued task."),
        false => println!("GitHub is not connected. Run: hq github connect"),
    }
    Ok(())
}

async fn connect(client: AppClient) -> Result<()> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .context("could not open a local port for GitHub's return")?;
    let base = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let flow = Arc::new(BrowserFlow::new(client, base));
    let link = flow.begin();
    let app: axum::Router = browser_routes(flow.clone());
    tokio::spawn(async move { axum::serve(listener, app).await });

    println!("Open this on this computer, press Create, then pick repositories and press Install:");
    println!("\n  {link}\n");
    println!("On a server, forward the port first (ssh -L <port>:127.0.0.1:<port> <server>).");
    println!("Choose \"Only select repositories\" on GitHub's page; it starts on \"All\".");
    println!("If the browser ends on an address that will not load, paste that address here.");
    open_in_browser(&link);

    tokio::select! {
        _ = flow.installed.notified() => {}
        pasted = read_line() => finish_pasted(&flow, &pasted?).await?,
    }
    let record = flow.client.refresh_installation().await?;
    print_standing(Some(&record));
    Ok(())
}

async fn read_line() -> Result<String> {
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await?;
    Ok(line.trim().to_string())
}

/// `code` and `state` from the address the browser could not load.
fn parse_return_address(text: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(text.trim()).ok()?;
    let find = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    Some((find("code")?, find("state")?))
}

async fn finish_pasted(flow: &BrowserFlow, pasted: &str) -> Result<()> {
    let (code, state) =
        parse_return_address(pasted).context("that address has no code and state in it")?;
    let record = flow.client.finish(&flow.flows, &state, &code).await?;
    println!(
        "App created. Install it, then press Enter here:\n\n  {}\n",
        record.install_url()
    );
    open_in_browser(&record.install_url());
    read_line().await?;
    Ok(())
}

fn open_in_browser(link: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // The printed link is the way that always works; a machine with no browser just skips this.
    let _ = std::process::Command::new(opener)
        .arg(link)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

async fn status(client: AppClient) -> Result<()> {
    if client.store().load().is_none() {
        print_standing(None);
        return Ok(());
    }
    match client.refresh_installation().await {
        Ok(record) => print_standing(Some(&record)),
        Err(e) if e.downcast_ref::<TokenRefused>().is_some() => {
            print_standing(client.store().load().as_ref());
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

fn print_standing(record: Option<&AppRecord>) {
    match (standing(record), record) {
        (Standing::Connected, Some(r)) => {
            let selection = r.repository_selection.as_deref().unwrap_or("unknown");
            println!(
                "Connected to {} through {} (repositories: {selection}).",
                r.owner_login, r.slug
            );
        }
        (Standing::NeedsInstall, Some(r)) => {
            println!("Created but not installed: {}", r.install_url())
        }
        (Standing::Reconnect, _) => {
            println!("GitHub no longer accepts this App. Reconnect GitHub: hq github connect");
        }
        _ => println!("GitHub is not connected. Run: hq github connect"),
    }
}

fn disconnect(client: &AppClient) -> Result<()> {
    match client.disconnect()? {
        Some(settings) => {
            println!(
                "The key is gone from this device. Delete the App on GitHub to cut its access:"
            );
            println!("\n  {settings}\n");
            open_in_browser(&settings);
        }
        None => println!("GitHub was not connected."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_return_address;

    #[test]
    fn a_pasted_address_gives_its_code_and_state() {
        let parsed =
            parse_return_address("http://127.0.0.1:9/github/connect/return?code=c1&state=s1");
        assert_eq!(parsed, Some(("c1".to_string(), "s1".to_string())));
    }

    #[test]
    fn an_address_missing_either_is_refused() {
        assert_eq!(parse_return_address("http://127.0.0.1:9/x?code=c1"), None);
        assert_eq!(parse_return_address("not an address"), None);
    }
}
