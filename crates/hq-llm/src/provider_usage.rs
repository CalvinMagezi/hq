//! What a provider says about its own spend and balance, one adapter per provider that has an
//! endpoint for it. Providers without one are read from HQ's own ledger instead; see
//! `docs/plans/tokenomics.md` for the verified capability matrix.

use std::time::Duration;

use chrono::{DateTime, Datelike, TimeZone, Utc};
use hq_core::config::BackendKind;
use serde::Serialize;
use serde_json::Value;

use crate::openrouter_usage::{OpenRouterUsage, fetch_usage};
use crate::provider::LlmError;

/// Per request, so a hung provider cannot hold the caller for the shared client's 300 s.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Anthropic's cost report returns at most 31 daily buckets per page; a month fits in one.
const COST_REPORT_PAGE_LIMIT: u32 = 31;
/// A month never needs more pages than this, so a looping `next_page` cannot hang the read.
const COST_REPORT_MAX_PAGES: usize = 4;
const CENTS_PER_DOLLAR: f64 = 100.0;
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Which way HQ learns a backend's spend.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Adapter {
    OpenRouter,
    Deepseek,
    Moonshot,
    /// Anthropic's organization cost report. Needs an admin key the owner opts into.
    AnthropicAdmin,
    /// A subscription quota read elsewhere (`/api/copilot-usage`).
    Subscription,
    /// Runs on the owner's hardware; nothing is billed.
    Local,
    /// The provider documents no balance or spend endpoint: HQ's own ledger is the only source.
    LedgerOnly,
}

/// Choose the adapter for a configured backend from its kind and endpoint.
pub fn adapter_for(kind: BackendKind, endpoint: Option<&str>, has_admin_key: bool) -> Adapter {
    let host = host_of(endpoint);
    match kind {
        BackendKind::GithubCopilotApi | BackendKind::GithubCopilotCli | BackendKind::KimiCode => {
            Adapter::Subscription
        }
        BackendKind::Openrouter => Adapter::OpenRouter,
        BackendKind::AnthropicCompatible if host == "api.anthropic.com" && has_admin_key => {
            Adapter::AnthropicAdmin
        }
        _ if host.contains("openrouter.ai") => Adapter::OpenRouter,
        _ if host == "api.deepseek.com" => Adapter::Deepseek,
        _ if host == "api.moonshot.ai" || host == "api.moonshot.cn" => Adapter::Moonshot,
        _ if host.contains("githubcopilot.com") => Adapter::Subscription,
        _ if is_loopback(&host) => Adapter::Local,
        _ => Adapter::LedgerOnly,
    }
}

fn host_of(endpoint: Option<&str>) -> String {
    endpoint
        .and_then(|e| e.split("://").nth(1))
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0")
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Balance {
    pub amount: f64,
    pub currency: String,
}

/// Spend over the current UTC day, Monday-based week and month, in USD.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Spend {
    pub today: Option<f64>,
    pub week: Option<f64>,
    pub month: Option<f64>,
}

/// One successful read of a provider's own figures.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ProviderReading {
    pub balance: Option<Balance>,
    pub spend: Option<Spend>,
    /// What is left of a spend cap on the key, when the provider has one.
    pub limit_remaining: Option<f64>,
}

async fn get_json(url: &str, headers: &[(&str, &str)], label: &str) -> Result<Value, LlmError> {
    let mut req = crate::http::SHARED_HTTP_CLIENT
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .header("Accept", "application/json");
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| LlmError::from_request_error(&e))?;
    let status = resp.status().as_u16();
    if matches!(status, 401 | 403) {
        return Err(LlmError::Auth {
            status,
            message: format!("{label} refused the key"),
        });
    }
    if !(200..300).contains(&status) {
        return Err(LlmError::ServerError {
            status,
            message: format!("{label} usage request failed"),
        });
    }
    resp.json()
        .await
        .map_err(|e| LlmError::Other(anyhow::anyhow!("{label} body was not JSON: {e}")))
}

fn bearer(key: &str) -> String {
    format!("Bearer {key}")
}

fn parse_money(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

// ─── OpenRouter ─────────────────────────────────────────────────────

pub fn openrouter_reading(u: &OpenRouterUsage) -> ProviderReading {
    ProviderReading {
        balance: u.credits_left().map(|amount| Balance {
            amount,
            currency: "USD".into(),
        }),
        spend: Some(Spend {
            today: u.usage_daily,
            week: u.usage_weekly,
            month: u.usage_monthly,
        }),
        limit_remaining: u.limit_remaining,
    }
}

pub async fn read_openrouter(base: &str, key: &str) -> Result<ProviderReading, LlmError> {
    fetch_usage(base, key).await.map(|u| openrouter_reading(&u))
}

// ─── DeepSeek ───────────────────────────────────────────────────────

/// `GET /user/balance`: `balance_infos[]` with `currency` and a string `total_balance`.
pub fn parse_deepseek_balance(body: &Value) -> Option<ProviderReading> {
    let infos = body.get("balance_infos")?.as_array()?;
    let info = infos
        .iter()
        .find(|i| i.get("currency").and_then(Value::as_str) == Some("USD"))
        .or_else(|| infos.first())?;
    Some(ProviderReading {
        balance: Some(Balance {
            amount: parse_money(info.get("total_balance")?)?,
            currency: info.get("currency")?.as_str()?.to_string(),
        }),
        ..ProviderReading::default()
    })
}

pub async fn read_deepseek(base: &str, key: &str) -> Result<ProviderReading, LlmError> {
    let url = format!("{}/user/balance", base.trim_end_matches('/'));
    let body = get_json(&url, &[("Authorization", &bearer(key))], "DeepSeek").await?;
    parse_deepseek_balance(&body).ok_or_else(|| {
        LlmError::Other(anyhow::anyhow!(
            "DeepSeek balance response had no balance_infos"
        ))
    })
}

// ─── Moonshot ───────────────────────────────────────────────────────

/// `GET /v1/users/me/balance`: `data.available_balance`, USD on the international platform.
pub fn parse_moonshot_balance(body: &Value, currency: &str) -> Option<ProviderReading> {
    if body
        .get("code")
        .and_then(Value::as_i64)
        .is_some_and(|c| c != 0)
    {
        return None;
    }
    let amount = parse_money(body.get("data")?.get("available_balance")?)?;
    Some(ProviderReading {
        balance: Some(Balance {
            amount,
            currency: currency.to_string(),
        }),
        ..ProviderReading::default()
    })
}

pub async fn read_moonshot(base: &str, key: &str) -> Result<ProviderReading, LlmError> {
    let root = base.trim_end_matches('/').trim_end_matches("/v1");
    let currency = if root.ends_with(".cn") { "CNY" } else { "USD" };
    let url = format!("{root}/v1/users/me/balance");
    let body = get_json(&url, &[("Authorization", &bearer(key))], "Moonshot").await?;
    parse_moonshot_balance(&body, currency).ok_or_else(|| {
        LlmError::Other(anyhow::anyhow!(
            "Moonshot balance response was not a success"
        ))
    })
}

// ─── Anthropic organization cost report ─────────────────────────────

/// Sum a cost report's daily buckets into UTC day, week and month totals in dollars.
///
/// Buckets carry `starting_at` (RFC 3339, UTC) and `results[].amount`, a decimal string in cents.
pub fn parse_cost_report(pages: &[Value], now: DateTime<Utc>) -> Spend {
    let day = now.date_naive();
    let week_start = day - chrono::Duration::days(i64::from(now.weekday().num_days_from_monday()));
    let month_start = day.with_day(1).unwrap_or(day);
    let mut spend = Spend {
        today: Some(0.0),
        week: Some(0.0),
        month: Some(0.0),
    };
    for bucket in pages
        .iter()
        .filter_map(|p| p.get("data")?.as_array())
        .flatten()
    {
        let Some(start) = bucket
            .get("starting_at")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc).date_naive())
        else {
            continue;
        };
        let dollars: f64 = bucket
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|r| parse_money(r.get("amount")?))
            .sum::<f64>()
            / CENTS_PER_DOLLAR;
        if start == day {
            spend.today = spend.today.map(|t| t + dollars);
        }
        if start >= week_start {
            spend.week = spend.week.map(|t| t + dollars);
        }
        if start >= month_start {
            spend.month = spend.month.map(|t| t + dollars);
        }
    }
    spend
}

pub async fn read_anthropic_admin(
    base: &str,
    admin_key: &str,
    now: DateTime<Utc>,
) -> Result<ProviderReading, LlmError> {
    let month_start = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    let root = base.trim_end_matches('/');
    let mut pages = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..COST_REPORT_MAX_PAGES {
        let mut url = format!(
            "{root}/organizations/cost_report?starting_at={}&bucket_width=1d&limit={COST_REPORT_PAGE_LIMIT}",
            month_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        if let Some(page) = &cursor {
            url.push_str(&format!("&page={page}"));
        }
        let body = get_json(
            &url,
            &[
                ("x-api-key", admin_key),
                ("anthropic-version", ANTHROPIC_VERSION),
            ],
            "Anthropic",
        )
        .await?;
        let next = body
            .get("has_more")
            .and_then(Value::as_bool)
            .filter(|more| *more)
            .and_then(|_| body.get("next_page").and_then(Value::as_str))
            .map(str::to_string);
        pages.push(body);
        match next {
            Some(page) => cursor = Some(page),
            None => break,
        }
    }
    Ok(ProviderReading {
        spend: Some(parse_cost_report(&pages, now)),
        ..ProviderReading::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn adapters_follow_the_endpoint_and_never_assume_an_api_that_is_not_there() {
        let a = |k, e, admin| adapter_for(k, Some(e), admin);
        assert_eq!(
            a(
                BackendKind::OpenaiCompatible,
                "https://openrouter.ai/api/v1",
                false
            ),
            Adapter::OpenRouter
        );
        assert_eq!(
            a(
                BackendKind::OpenaiCompatible,
                "https://api.deepseek.com/v1",
                false
            ),
            Adapter::Deepseek
        );
        assert_eq!(
            a(
                BackendKind::OpenaiCompatible,
                "https://api.moonshot.ai/v1",
                false
            ),
            Adapter::Moonshot
        );
        assert_eq!(
            a(
                BackendKind::OpenaiCompatible,
                "http://localhost:11434/v1",
                false
            ),
            Adapter::Local
        );
        assert_eq!(
            a(
                BackendKind::OpenaiCompatible,
                "https://api.groq.com/openai/v1",
                false
            ),
            Adapter::LedgerOnly
        );
        assert_eq!(
            a(
                BackendKind::KimiCode,
                "https://api.kimi.com/coding/v1",
                false
            ),
            Adapter::Subscription
        );
        let anthropic = "https://api.anthropic.com/v1";
        assert_eq!(
            a(BackendKind::AnthropicCompatible, anthropic, false),
            Adapter::LedgerOnly
        );
        assert_eq!(
            a(BackendKind::AnthropicCompatible, anthropic, true),
            Adapter::AnthropicAdmin
        );
    }

    #[test]
    fn deepseek_balance_prefers_usd_and_reads_the_string_amount() {
        let body = json!({"is_available": true, "balance_infos": [
            {"currency": "CNY", "total_balance": "100.00"},
            {"currency": "USD", "total_balance": "12.34", "granted_balance": "2.00"}
        ]});
        let r = parse_deepseek_balance(&body).unwrap();
        assert_eq!(
            r.balance,
            Some(Balance {
                amount: 12.34,
                currency: "USD".into()
            })
        );
        assert!(parse_deepseek_balance(&json!({})).is_none());
    }

    #[test]
    fn moonshot_balance_rejects_a_non_zero_code() {
        let ok = json!({"code": 0, "data": {"available_balance": 49.5, "cash_balance": 40.0}});
        assert_eq!(
            parse_moonshot_balance(&ok, "USD")
                .unwrap()
                .balance
                .unwrap()
                .amount,
            49.5
        );
        assert!(parse_moonshot_balance(&json!({"code": 1, "data": {}}), "USD").is_none());
    }

    #[test]
    fn the_cost_report_is_summed_into_utc_periods_and_cents_become_dollars() {
        // 2026-10-14 is a Wednesday, so the week began on Monday the 12th.
        let now = at("2026-10-14T15:00:00Z");
        let day = |d: &str, cents: &[&str]| {
            json!({"starting_at": format!("{d}T00:00:00Z"),
                   "results": cents.iter().map(|c| json!({"amount": c})).collect::<Vec<_>>()})
        };
        let page = json!({"data": [
            day("2026-10-01", &["1000"]),
            day("2026-10-11", &["500"]),
            day("2026-10-12", &["200", "50.5"]),
            day("2026-10-14", &["300"]),
        ]});
        let s = parse_cost_report(&[page], now);
        assert_eq!(s.today, Some(3.0));
        assert_eq!(s.week, Some(5.505));
        assert_eq!(s.month, Some(20.505));
    }

    #[tokio::test]
    async fn deepseek_is_read_with_the_bearer_key_and_a_refusal_is_an_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/balance"))
            .and(header("authorization", "Bearer k"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"balance_infos": [{"currency": "USD", "total_balance": "3.5"}]}),
            ))
            .mount(&server)
            .await;
        let r = read_deepseek(&server.uri(), "k").await.unwrap();
        assert_eq!(r.balance.unwrap().amount, 3.5);
        let e = read_deepseek(&server.uri(), "other").await.unwrap_err();
        assert!(matches!(e, LlmError::ServerError { status: 404, .. }));
    }

    #[tokio::test]
    async fn a_401_from_the_provider_maps_to_an_auth_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let e = read_deepseek(&server.uri(), "k").await.unwrap_err();
        assert!(matches!(e, LlmError::Auth { status: 401, .. }));
        let e = read_moonshot(&server.uri(), "k").await.unwrap_err();
        assert!(matches!(e, LlmError::Auth { status: 401, .. }));
    }

    #[tokio::test]
    async fn the_anthropic_report_follows_pages_with_the_admin_key_headers() {
        let server = MockServer::start().await;
        let bucket = |d: &str, c: &str| json!({"starting_at": format!("{d}T00:00:00Z"), "results": [{"amount": c}]});
        Mock::given(method("GET"))
            .and(path("/organizations/cost_report"))
            .and(header("x-api-key", "adm"))
            .and(header("anthropic-version", ANTHROPIC_VERSION))
            .and(query_param("page", "p2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"data": [bucket("2026-10-14", "250")], "has_more": false}),
                ),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/organizations/cost_report"))
            .and(header("x-api-key", "adm"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [bucket("2026-10-02", "100")], "has_more": true, "next_page": "p2"
            })))
            .mount(&server)
            .await;
        let r = read_anthropic_admin(&server.uri(), "adm", at("2026-10-14T09:00:00Z"))
            .await
            .unwrap();
        let s = r.spend.unwrap();
        assert_eq!((s.today, s.month), (Some(2.5), Some(3.5)));
    }
}
