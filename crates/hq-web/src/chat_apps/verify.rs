//! Checks a bot token against Telegram or Discord before HQ saves it.
//!
//! A Telegram token sits in the request path (`/bot<token>/getMe`), and `reqwest::Error`
//! prints the URL, so no outbound error text ever reaches a response or a log line with it.

use std::time::Duration;

use serde::Deserialize;

use super::Platform;

pub(crate) const TELEGRAM_API: &str = "https://api.telegram.org";
pub(crate) const DISCORD_API: &str = "https://discord.com/api/v10";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

const TELEGRAM_ID_DIGITS: std::ops::RangeInclusive<usize> = 5..=20;
const TELEGRAM_SECRET_CHARS: std::ops::RangeInclusive<usize> = 30..=64;
const DISCORD_TOKEN_CHARS: std::ops::RangeInclusive<usize> = 50..=100;
const DISCORD_TOKEN_PARTS: usize = 3;
/// Application flags for the Message Content intent: approved (verified apps) and limited
/// (unverified apps that switched it on in the developer portal).
const GATEWAY_MESSAGE_CONTENT: u64 = 1 << 18;
const GATEWAY_MESSAGE_CONTENT_LIMITED: u64 = 1 << 19;

/// What the platform says about the bot behind a token. Never holds the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verified {
    pub username: String,
    pub application_id: Option<String>,
    pub message_content: Option<bool>,
}

fn token_chars_ok(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Rejects anything that cannot be a bot token, before any outbound call.
pub(crate) fn check_shape(platform: Platform, token: &str) -> Result<(), &'static str> {
    let ok = match platform {
        Platform::Telegram => token.split_once(':').is_some_and(|(id, secret)| {
            TELEGRAM_ID_DIGITS.contains(&id.len())
                && id.chars().all(|c| c.is_ascii_digit())
                && TELEGRAM_SECRET_CHARS.contains(&secret.len())
                && token_chars_ok(secret)
        }),
        Platform::Discord => {
            let parts: Vec<&str> = token.split('.').collect();
            DISCORD_TOKEN_CHARS.contains(&token.len())
                && parts.len() == DISCORD_TOKEN_PARTS
                && parts.iter().all(|p| !p.is_empty() && token_chars_ok(p))
        }
    };
    if ok {
        return Ok(());
    }
    Err(match platform {
        Platform::Telegram => {
            "That does not look like a Telegram bot token. It is a number, a colon, then about 35 letters and digits."
        }
        Platform::Discord => {
            "That does not look like a Discord bot token. Copy it from the Bot page with Reset Token."
        }
    })
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(VERIFY_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "HQ could not set up an outbound connection.".to_string())
}

fn unreachable(service: &str, e: reqwest::Error) -> String {
    tracing::warn!(service, error = %e.without_url(), "chat apps: token check failed to connect");
    format!("HQ could not reach {service}. Check the server's internet connection and try again.")
}

fn refused(service: &str) -> String {
    format!("{service} did not accept that token. Copy it again and paste the whole thing.")
}

fn unreadable(service: &str) -> String {
    format!("{service} answered with something HQ could not read. Try again in a minute.")
}

async fn get_json<T: serde::de::DeserializeOwned>(
    req: reqwest::RequestBuilder,
    service: &str,
) -> Result<T, String> {
    let res = req.send().await.map_err(|e| unreachable(service, e))?;
    let status = res.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::NOT_FOUND {
        return Err(refused(service));
    }
    if !status.is_success() {
        return Err(unreadable(service));
    }
    res.json::<T>().await.map_err(|_| unreadable(service))
}

#[derive(Deserialize)]
struct TelegramReply {
    ok: bool,
    result: Option<TelegramBot>,
}

#[derive(Deserialize)]
struct TelegramBot {
    username: Option<String>,
}

async fn verify_telegram(base: &str, token: &str) -> Result<Verified, String> {
    const SERVICE: &str = "Telegram";
    let req = client()?.get(format!("{base}/bot{token}/getMe"));
    let reply: TelegramReply = get_json(req, SERVICE).await?;
    let username = reply
        .result
        .and_then(|b| b.username)
        .filter(|_| reply.ok)
        .ok_or_else(|| unreadable(SERVICE))?;
    Ok(Verified {
        username,
        application_id: None,
        message_content: None,
    })
}

#[derive(Deserialize)]
struct DiscordUser {
    username: String,
}

#[derive(Deserialize)]
struct DiscordApplication {
    id: String,
    #[serde(default)]
    flags: Option<u64>,
}

async fn verify_discord(base: &str, token: &str) -> Result<Verified, String> {
    const SERVICE: &str = "Discord";
    let http = client()?;
    let auth = format!("Bot {token}");
    let user: DiscordUser = get_json(
        http.get(format!("{base}/users/@me"))
            .header(reqwest::header::AUTHORIZATION, &auth),
        SERVICE,
    )
    .await?;
    let app: DiscordApplication = get_json(
        http.get(format!("{base}/oauth2/applications/@me"))
            .header(reqwest::header::AUTHORIZATION, &auth),
        SERVICE,
    )
    .await?;
    let intent = GATEWAY_MESSAGE_CONTENT | GATEWAY_MESSAGE_CONTENT_LIMITED;
    Ok(Verified {
        username: user.username,
        application_id: Some(app.id),
        message_content: app.flags.map(|f| f & intent != 0),
    })
}

/// Asks the platform who the bot is. `base` is the API root, so tests can point it at a local server.
pub(crate) async fn verify(
    platform: Platform,
    base: &str,
    token: &str,
) -> Result<Verified, String> {
    match platform {
        Platform::Telegram => verify_telegram(base, token).await,
        Platform::Discord => verify_discord(base, token).await,
    }
}

/// The real API root for a platform.
pub(crate) fn api_base(platform: Platform) -> &'static str {
    match platform {
        Platform::Telegram => TELEGRAM_API,
        Platform::Discord => DISCORD_API,
    }
}
