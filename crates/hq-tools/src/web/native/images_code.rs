//! Engines for the `images` and `code` categories. All are public JSON APIs
//! that need no key, so their queries are sent as written: the `site:`
//! operators `effective_query` adds mean nothing to them, and the domain
//! post-filter still applies.

use super::super::*;
use super::engines::{Engine, freshness_secs, result};
use super::markup::{collapse_ws, html_text};

/// A longer image URL would be cut mid-way by the snippet cap, so such a hit is dropped.
const MAX_IMAGE_URL_CHARS: usize = 300;

/// Third-party URLs reach an agent that may fetch them, so only http(s) ones pass.
fn web_url(url: &str) -> Option<String> {
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_string())
}

pub(super) async fn fetch(
    engine: Engine,
    client: &Client,
    base: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let q = query.trim().to_string();
    let per_page = opts.max_results.clamp(1, MAX_RESULTS_CAP);
    let page = opts.page;
    let offset = (page.saturating_sub(1) as usize * per_page).to_string();
    let since = opts
        .freshness
        .map(|f| chrono::Utc::now() - chrono::Duration::seconds(freshness_secs(f)));
    match engine {
        Engine::CommonsImages => {
            let params = [
                ("action", "query".to_string()),
                ("generator", "search".into()),
                ("gsrnamespace", "6".into()),
                ("gsrsearch", q),
                ("gsrlimit", per_page.to_string()),
                ("gsroffset", offset),
                ("prop", "imageinfo".into()),
                ("iiprop", "url|size|mime|extmetadata".into()),
                ("format", "json".into()),
                ("utf8", "1".into()),
            ];
            get_json(client.get(format!("{base}/w/api.php")).query(&params)).await
        }
        Engine::Openverse => {
            let params = [
                ("q", q),
                ("page_size", per_page.to_string()),
                ("page", page.to_string()),
            ];
            get_json(client.get(format!("{base}/v1/images/")).query(&params)).await
        }
        Engine::GitHub => {
            let q = match since {
                Some(d) => format!("{q} pushed:>{}", d.format("%Y-%m-%d")),
                None => q,
            };
            let params = [
                ("q", q),
                ("per_page", per_page.to_string()),
                ("page", page.to_string()),
            ];
            let req = client
                .get(format!("{base}/search/repositories"))
                .header("Accept", "application/vnd.github+json")
                .query(&params);
            get_json(req).await
        }
        Engine::StackOverflow | Engine::AskUbuntu | Engine::SuperUser => {
            let site = match engine {
                Engine::AskUbuntu => "askubuntu",
                Engine::SuperUser => "superuser",
                _ => "stackoverflow",
            };
            let mut params = vec![
                ("order", "desc".to_string()),
                ("sort", "relevance".into()),
                ("q", q),
                ("site", site.into()),
                ("pagesize", per_page.to_string()),
                ("page", page.to_string()),
            ];
            if let Some(d) = since {
                params.push(("fromdate", d.timestamp().to_string()));
            }
            get_json(
                client
                    .get(format!("{base}/2.3/search/advanced"))
                    .query(&params),
            )
            .await
        }
        Engine::Crates => {
            let params = [
                ("q", q),
                ("per_page", per_page.to_string()),
                ("page", page.to_string()),
            ];
            get_json(client.get(format!("{base}/api/v1/crates")).query(&params)).await
        }
        Engine::Npm => {
            let params = [
                ("text", q),
                ("size", per_page.to_string()),
                ("from", offset),
            ];
            get_json(client.get(format!("{base}/-/v1/search")).query(&params)).await
        }
        Engine::Mdn => {
            let params = [("q", q), ("page", page.to_string())];
            get_json(client.get(format!("{base}/api/v1/search")).query(&params)).await
        }
        Engine::EuropePmc => {
            let params = [
                ("query", q),
                ("format", "json".to_string()),
                ("resultType", "core".into()),
                ("pageSize", per_page.to_string()),
                ("page", page.to_string()),
            ];
            get_json(
                client
                    .get(format!("{base}/europepmc/webservices/rest/search"))
                    .query(&params),
            )
            .await
        }
        _ => Err(ProviderError::new("not an images or code engine")),
    }
}

pub(super) fn parse(engine: Engine, body: &Value) -> Result<Vec<SearchResult>, String> {
    match engine {
        Engine::CommonsImages => parse_commons(body),
        Engine::Openverse => parse_openverse(body),
        Engine::GitHub => parse_github(body),
        Engine::StackOverflow | Engine::AskUbuntu | Engine::SuperUser => parse_stack_overflow(body),
        Engine::Crates => parse_crates(body),
        Engine::Npm => parse_npm(body),
        Engine::Mdn => parse_mdn(body),
        Engine::EuropePmc => parse_europe_pmc(body),
        _ => Err("not an images or code engine".into()),
    }
}

/// `Image: <direct url> | 1024x685 | CC BY | Artist` then any description. The
/// direct image URL leads so it survives the snippet length cap.
pub(super) fn image_snippet(
    image_url: &str,
    dims: Option<(u64, u64)>,
    license: &str,
    creator: &str,
    about: &str,
) -> String {
    let mut parts = vec![format!("Image: {image_url}")];
    if let Some((w, h)) = dims {
        parts.push(format!("{w}x{h}"));
    }
    parts.extend(
        [license, creator]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(String::from),
    );
    let head = parts.join(" | ");
    if about.is_empty() {
        head
    } else {
        format!("{head}. {about}")
    }
}

fn parse_commons(body: &Value) -> Result<Vec<SearchResult>, String> {
    let Some(pages) = body["query"]["pages"].as_object() else {
        // The API omits `query` entirely when nothing matched.
        return Ok(Vec::new());
    };
    let mut pages: Vec<&Value> = pages.values().collect();
    pages.sort_by_key(|p| p["index"].as_u64().unwrap_or(u64::MAX));
    Ok(pages
        .into_iter()
        .filter_map(|p| {
            let info = p["imageinfo"].as_array()?.first()?;
            // The search also matches video, audio and documents.
            if !info["mime"]
                .as_str()
                .is_some_and(|m| m.starts_with("image/"))
            {
                return None;
            }
            let page_url = non_empty_str(&info["descriptionurl"]).and_then(|u| web_url(&u))?;
            let image_url = non_empty_str(&info["url"])
                .and_then(|u| web_url(&u))
                .filter(|u| u.len() <= MAX_IMAGE_URL_CHARS)?;
            let title = p["title"].as_str()?.trim_start_matches("File:").to_string();
            let meta = |key: &str| {
                html_text(
                    info["extmetadata"][key]["value"]
                        .as_str()
                        .unwrap_or_default(),
                )
            };
            let dims = info["width"].as_u64().zip(info["height"].as_u64());
            let snippet = image_snippet(
                &image_url,
                dims,
                &meta("LicenseShortName"),
                &meta("Artist"),
                &meta("ImageDescription"),
            );
            Some(result(title, page_url, snippet))
        })
        .collect())
}

fn parse_openverse(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["results"]
        .as_array()
        .ok_or("response has no `results` array")?;
    Ok(items
        .iter()
        .filter_map(|r| {
            let image_url = non_empty_str(&r["url"])
                .and_then(|u| web_url(&u))
                .filter(|u| u.len() <= MAX_IMAGE_URL_CHARS)?;
            let page_url = non_empty_str(&r["foreign_landing_url"])
                .and_then(|u| web_url(&u))
                .unwrap_or_else(|| image_url.clone());
            let title = non_empty_str(&r["title"]).unwrap_or_else(|| page_url.clone());
            let license = match (r["license"].as_str(), r["license_version"].as_str()) {
                (Some(l @ ("cc0" | "pdm")), _) => l.to_uppercase(),
                (Some(l), Some(v)) => format!("CC {} {v}", l.to_uppercase()),
                (Some(l), None) => format!("CC {}", l.to_uppercase()),
                _ => String::new(),
            };
            let dims = r["width"].as_u64().zip(r["height"].as_u64());
            let snippet = image_snippet(
                &image_url,
                dims,
                &license,
                r["creator"].as_str().unwrap_or_default(),
                "",
            );
            Some(result(title, page_url, snippet))
        })
        .collect())
}

fn parse_github(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["items"]
        .as_array()
        .ok_or("response has no `items` array")?;
    Ok(items
        .iter()
        .filter_map(|r| {
            let name = r["full_name"].as_str()?;
            let url = non_empty_str(&r["html_url"]).and_then(|u| web_url(&u))?;
            let about = r["description"].as_str().unwrap_or_default();
            let lang = r["language"]
                .as_str()
                .map(|l| format!("{l}, "))
                .unwrap_or_default();
            let snippet = format!(
                "{about} ({lang}{} stars)",
                r["stargazers_count"].as_u64().unwrap_or(0)
            );
            let mut hit = result(name.to_string(), url, snippet);
            hit.published = non_empty_str(&r["pushed_at"]);
            Some(hit)
        })
        .collect())
}

fn parse_stack_overflow(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["items"]
        .as_array()
        .ok_or("response has no `items` array")?;
    Ok(items
        .iter()
        .filter_map(|q| {
            let title = html_text(q["title"].as_str()?);
            let url = non_empty_str(&q["link"]).and_then(|u| web_url(&u))?;
            let tags: Vec<&str> = q["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let answered = if q["is_answered"].as_bool().unwrap_or(false) {
                "answered"
            } else {
                "unanswered"
            };
            let snippet = format!(
                "{} votes, {} answers ({answered}); tags: {}",
                q["score"].as_i64().unwrap_or(0),
                q["answer_count"].as_u64().unwrap_or(0),
                tags.join(", ")
            );
            let mut hit = result(title, url, snippet);
            hit.published = q["creation_date"]
                .as_i64()
                .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                .map(|d| d.to_rfc3339());
            Some(hit)
        })
        .collect())
}

fn parse_crates(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["crates"]
        .as_array()
        .ok_or("response has no `crates` array")?;
    Ok(items
        .iter()
        .filter_map(|c| {
            let name = c["name"].as_str()?;
            let about = collapse_ws(c["description"].as_str().unwrap_or_default());
            let snippet = format!(
                "{about} (v{}, {} downloads)",
                c["max_version"].as_str().unwrap_or("?"),
                c["downloads"].as_u64().unwrap_or(0)
            );
            let mut hit = result(
                format!("{name} (crates.io)"),
                format!("https://crates.io/crates/{name}"),
                snippet,
            );
            hit.published = non_empty_str(&c["updated_at"]);
            Some(hit)
        })
        .collect())
}

fn parse_npm(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["objects"]
        .as_array()
        .ok_or("response has no `objects` array")?;
    Ok(items
        .iter()
        .filter_map(|o| {
            let p = &o["package"];
            let name = p["name"].as_str()?;
            let url = non_empty_str(&p["links"]["npm"])
                .and_then(|u| web_url(&u))
                .unwrap_or_else(|| format!("https://www.npmjs.com/package/{name}"));
            let snippet = format!(
                "{} (v{})",
                collapse_ws(p["description"].as_str().unwrap_or_default()),
                p["version"].as_str().unwrap_or("?")
            );
            let mut hit = result(format!("{name} (npm)"), url, snippet);
            hit.published = non_empty_str(&p["date"]);
            Some(hit)
        })
        .collect())
}

fn parse_mdn(body: &Value) -> Result<Vec<SearchResult>, String> {
    let docs = body["documents"]
        .as_array()
        .ok_or("response has no `documents` array")?;
    Ok(docs
        .iter()
        .filter_map(|d| {
            let path = d["mdn_url"].as_str().filter(|p| p.starts_with('/'))?;
            let title = d["title"].as_str()?;
            let url = format!("https://developer.mozilla.org{path}");
            Some(result(
                title.to_string(),
                url,
                d["summary"].as_str().unwrap_or_default().to_string(),
            ))
        })
        .collect())
}

fn parse_europe_pmc(body: &Value) -> Result<Vec<SearchResult>, String> {
    let items = body["resultList"]["result"]
        .as_array()
        .ok_or("response has no `resultList.result` array")?;
    Ok(items
        .iter()
        .filter_map(|r| {
            let (id, source) = (r["id"].as_str()?, r["source"].as_str()?);
            let title = html_text(r["title"].as_str()?);
            let url = format!("https://europepmc.org/article/{source}/{id}");
            let journal = r["journalTitle"].as_str().unwrap_or_default();
            let year = r["pubYear"].as_str().unwrap_or_default();
            let authors = r["authorString"].as_str().unwrap_or_default();
            let about = html_text(r["abstractText"].as_str().unwrap_or_default());
            let head = [authors, journal, year]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" | ");
            let snippet = if about.is_empty() {
                head
            } else {
                format!("{head}. {about}")
            };
            let mut hit = result(title, url, snippet);
            hit.published = non_empty_str(&r["firstPublicationDate"])
                .or_else(|| (!year.is_empty()).then(|| year.to_string()));
            Some(hit)
        })
        .collect())
}
