//! Tier 2 lyrics source: lrclib.net (free, no auth, synced LRC).
//!
//! `/api/search` instead of `/api/get`: exact-match requests fail on remasters,
//! featured artists and "feat." suffixes, so we search and then pick the candidate
//! whose duration is closest to what the player reports. Duration is the single most
//! reliable discriminator between a studio cut, a live version and a cover.

use super::Query;
use serde::Deserialize;
use std::time::Duration;

const ENDPOINT: &str = "https://lrclib.net/api/search";

fn user_agent() -> String {
    format!("LyricFloat/{} (desktop lyrics overlay)", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Deserialize)]
struct Hit {
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

/// Returns the best synced LRC text for a query, if any.
pub fn fetch_synced(query: &Query) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(user_agent())
        // Cloudflare + rustls over h2 has been observed to break the body stream
        // mid-flight; HTTP/1.1 is boring and reliable here.
        .http1_only()
        .build()
        .ok()?;

    let url = format!(
        "{}?track_name={}&artist_name={}",
        ENDPOINT,
        urlencoding::encode(&query.title),
        urlencoding::encode(&query.artist)
    );

    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(err) => {
            eprintln!("[lrclib] request failed: {err}");
            return None;
        }
    };
    let status = response.status();
    if !status.is_success() {
        eprintln!("[lrclib] HTTP {status} for {url}");
        return None;
    }
    eprintln!(
        "[lrclib] GET {url} -> {status} enc={:?} len={:?} te={:?}",
        response.headers().get("content-encoding"),
        response.headers().get("content-length"),
        response.headers().get("transfer-encoding"),
    );
    let bytes = match response.bytes() {
        Ok(b) => b,
        Err(err) => {
            eprintln!("[lrclib] cannot read body: {err}");
            return None;
        }
    };
    let head_bytes: Vec<String> = bytes.iter().take(16).map(|b| format!("{b:02x}")).collect();
    eprintln!("[lrclib] body {} bytes, head={}", bytes.len(), head_bytes.join(" "));
    let body = String::from_utf8_lossy(&bytes);
    let hits: Vec<Hit> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(err) => {
            let head: String = body.chars().take(200).collect();
            eprintln!("[lrclib] cannot decode body ({err}); head={head:?}");
            return None;
        }
    };
    eprintln!("[lrclib] {} hits for '{}' / '{}'", hits.len(), query.title, query.artist);

    let mut best: Option<(&Hit, f64)> = None;
    for hit in hits.iter() {
        if hit.instrumental || hit.synced_lyrics.is_none() {
            continue;
        }
        // Title must plausibly match; lrclib search is fuzzy.
        if !hit.track_name.to_lowercase().contains(&query.title.to_lowercase())
            && !query.title.to_lowercase().contains(&hit.track_name.to_lowercase())
        {
            continue;
        }
        let delta = if hit.duration > 0.0 && query.duration_secs > 0.0 {
            (hit.duration - query.duration_secs).abs()
        } else {
            f64::MAX / 2.0
        };
        if delta > 8.0 {
            continue; // more than 8s off: almost certainly a different version
        }
        let better = match best {
            None => true,
            Some((_, best_delta)) => delta < best_delta,
        };
        if better {
            best = Some((hit, delta));
        }
    }

    let (hit, delta) = best?;
    eprintln!(
        "[lrclib] matched '{}' - '{}' (delta {:.1}s)",
        hit.track_name, hit.artist_name, delta
    );
    hit.synced_lyrics.clone()
}
