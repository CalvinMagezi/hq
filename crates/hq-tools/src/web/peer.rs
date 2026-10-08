//! Search through another HQ. When this machine's engines are blocked (a
//! datacenter address, for instance), a peer on a better network can answer:
//! its `web_search` is called over MCP, through the `remote_mcp` entry named by
//! `web_search_peer`. The peer's results are untrusted like any engine's and
//! pass the same sanitiser. A call carries `peer_hop`, which stops the peer
//! from forwarding it on, so two HQs naming each other cannot loop.

use super::*;
use hq_core::config::RemoteMcpServer;

pub(super) const PEER_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const PEER_HOP_ARG: &str = "peer_hop";

static PEER: std::sync::RwLock<Option<RemoteMcpServer>> = std::sync::RwLock::new(None);

/// Set once at startup from `web_search_peer`; `None` turns peer search off.
pub fn set_search_peer(server: Option<RemoteMcpServer>) {
    if let Ok(mut peer) = PEER.write() {
        *peer = server;
    }
}

pub(super) fn configured() -> Option<RemoteMcpServer> {
    PEER.read().ok()?.clone()
}

fn category_name(c: Category) -> &'static str {
    match c {
        Category::General => "general",
        Category::News => "news",
        Category::Science => "science",
        Category::Images => "images",
        Category::Code => "code",
    }
}

fn freshness_name(f: Freshness) -> &'static str {
    match f {
        Freshness::Day => "day",
        Freshness::Week => "week",
        Freshness::Month => "month",
        Freshness::Year => "year",
    }
}

/// The arguments for the peer's own `web_search` tool.
pub(super) fn request_args(query: &str, opts: &SearchOptions) -> Value {
    let mut args = json!({
        "query": query,
        "max_results": opts.max_results,
        "page": opts.page,
        PEER_HOP_ARG: true,
    });
    let Some(map) = args.as_object_mut() else {
        return args;
    };
    if let Some(f) = opts.freshness {
        map.insert("freshness".into(), freshness_name(f).into());
    }
    if let Some(c) = opts.category {
        map.insert("category".into(), category_name(c).into());
    }
    for (key, value) in [("language", &opts.language), ("country", &opts.country)] {
        if let Some(v) = value {
            map.insert(key.into(), v.clone().into());
        }
    }
    for (key, list) in [
        ("include_domains", &opts.include_domains),
        ("exclude_domains", &opts.exclude_domains),
    ] {
        if !list.is_empty() {
            map.insert(key.into(), json!(list));
        }
    }
    args
}

/// Ask the peer through its MCP gateway (`hq_call`), which is how HQ exposes tools.
pub(super) async fn request(
    server: &RemoteMcpServer,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let call = json!({"tool": "web_search", "args": request_args(query, opts)});
    crate::remote_mcp::call_named_server(server, "hq_call", call)
        .await
        .map_err(|e| ProviderError::new(format!("peer {}: {e}", server.name)))
}

pub(super) fn parse(json: &Value) -> Result<Page, String> {
    let items = json["results"]
        .as_array()
        .ok_or("the peer's answer has no `results` array")?;
    let results = items
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str().filter(|u| u.starts_with("http"))?.to_string();
            Some(SearchResult {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                snippet: r["snippet"].as_str().unwrap_or_default().to_string(),
                position: r["position"].as_u64().unwrap_or(0) as usize,
                provider: "peer".into(),
                domain: domain_of(&url),
                published: r["published"].as_str().map(str::to_string),
                engines: r["engines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.as_str().map(str::to_string))
                    .collect(),
                flagged: false,
                url,
            })
        })
        .collect();
    Ok(Page {
        results,
        next_page: json["next_page"].as_u64().and_then(|n| u32::try_from(n).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_carries_the_hop_flag_and_every_filter() {
        let opts = SearchOptions {
            freshness: Some(Freshness::Week),
            category: Some(Category::Code),
            language: Some("en".into()),
            include_domains: vec!["docs.rs".into()],
            ..SearchOptions::default()
        };
        let args = request_args("tokio", &opts);
        assert_eq!(args[PEER_HOP_ARG], true);
        assert_eq!(args["freshness"], "week");
        assert_eq!(args["category"], "code");
        assert_eq!(args["include_domains"][0], "docs.rs");
        assert!(args.get("country").is_none());
        assert!(SearchOptions::from_args(&args).is_ok(), "the peer must accept what we send");
    }

    #[test]
    fn a_peers_answer_becomes_results_and_junk_is_dropped() {
        let page = parse(&json!({"results": [
            {"title": "T", "url": "https://a.example/", "snippet": "s", "engines": ["mojeek"], "position": 1},
            {"title": "no link"},
            {"title": "bad scheme", "url": "javascript:alert(1)"},
        ], "next_page": 2}))
        .unwrap();
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].provider, "peer");
        assert_eq!(page.results[0].engines, ["mojeek"]);
        assert_eq!(page.next_page, Some(2));
        assert!(parse(&json!({"error": "x"})).is_err());
    }
}
