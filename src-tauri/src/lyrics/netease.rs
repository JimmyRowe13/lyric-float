//! Tier 2a: NetEase Cloud Music (music.163.com) lyric lookup.
//!
//! Why this tier exists: lrclib.net sits behind Cloudflare and is intermittently
//! unreachable from mainland networks (observed: 200 OK one minute, TCP connect
//! timeout the next). NetEase's public search + lyric endpoints answer reliably
//! there, and carry the Chinese repertoire that matters for this user.
//!
//! Both endpoints are unauthenticated and require only a browser-ish UA plus a
//! `Referer`, which is why this is a plain blocking HTTP call and not a service.

use super::Query;
use serde::Deserialize;
use std::time::Duration;

const SEARCH: &str = "https://music.163.com/api/search/get/web";
const LYRIC: &str = "https://music.163.com/api/song/lyric";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

#[derive(Debug, Deserialize)]
struct SearchResp {
    result: Option<SearchResult>,
}

#[derive(Debug, Deserialize)]
struct SearchResult {
    songs: Option<Vec<Song>>,
}

#[derive(Debug, Deserialize)]
struct Song {
    id: i64,
    name: String,
    #[serde(default)]
    artists: Vec<Artist>,
    /// milliseconds
    #[serde(default)]
    duration: f64,
}

#[derive(Debug, Deserialize)]
struct Artist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LyricResp {
    lrc: Option<LrcBody>,
}

#[derive(Debug, Deserialize)]
struct LrcBody {
    lyric: Option<String>,
}

fn client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(UA)
        .build()
        .ok()
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')' && *c != '（' && *c != '）')
        .collect::<String>()
        .to_lowercase()
}

/// Returns a score when the candidate plausibly is the track we are playing.
fn score(song: &Song, query: &Query) -> Option<i32> {
    let want_title = norm(&query.title);
    let got_title = norm(&song.name);
    let title_exact = got_title == want_title;
    let title_ok = got_title.contains(&want_title) || want_title.contains(&got_title);
    if !title_ok {
        return None;
    }

    let want_artist = norm(&query.artist);
    let artist_ok = !want_artist.is_empty()
        && song
            .artists
            .iter()
            .any(|a| norm(&a.name).contains(&want_artist) || want_artist.contains(&norm(&a.name)));

    let duration_delta = if song.duration > 0.0 && query.duration_secs > 0.0 {
        (song.duration / 1000.0 - query.duration_secs).abs()
    } else {
        f64::MAX / 2.0
    };

    // Artist is the reliable discriminator. A different artist is only acceptable
    // when the title matches exactly *and* the length is within a second — that is
    // a re-upload of the same recording, not a cover with its own timing.
    let same_recording = title_exact && duration_delta <= 1.0;
    if !artist_ok && !same_recording {
        return None;
    }

    let mut points = 0;
    if title_exact {
        points += 100;
    }
    if artist_ok {
        points += 60;
    }
    if duration_delta < 1.5 {
        points += 30;
    } else if duration_delta < 4.0 {
        points += 10;
    }
    Some(points - duration_delta.min(60.0) as i32)
}

pub fn fetch_synced(query: &Query) -> Option<String> {
    let client = client()?;

    let form = [
        ("s", format!("{} {}", query.title, query.artist)),
        ("type", "1".to_string()),
        ("offset", "0".to_string()),
        ("total", "true".to_string()),
        ("limit", "20".to_string()),
    ];

    let response = match client
        .post(SEARCH)
        .header("Referer", "https://music.163.com/")
        .header("Cookie", "appver=2.0.2")
        .form(&form)
        .send()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("[netease] search failed: {err}");
            return None;
        }
    };

    let body = response.text().unwrap_or_default();
    let parsed: SearchResp = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("[netease] search decode failed ({err}); head={:?}", &body.chars().take(120).collect::<String>());
            return None;
        }
    };

    let songs = parsed.result.and_then(|r| r.songs).unwrap_or_default();
    let mut best: Option<(i32, &Song)> = None;
    for song in songs.iter() {
        if let Some(points) = score(song, query) {
            if best.map(|(b, _)| points > b).unwrap_or(true) {
                best = Some((points, song));
            }
        }
    }

    let (points, song) = match best {
        Some(v) => v,
        None => {
            eprintln!("[netease] no plausible match among {} results", songs.len());
            return None;
        }
    };

    eprintln!(
        "[netease] picked id={} '{}' / '{}' (score {points})",
        song.id,
        song.name,
        song.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(",")
    );

    let lyric_url = format!("{LYRIC}?id={}&lv=1&kv=1&tv=-1", song.id);
    let response = match client
        .get(&lyric_url)
        .header("Referer", "https://music.163.com/")
        .header("Cookie", "appver=2.0.2")
        .send()
    {
        Ok(r) => r,
        Err(err) => {
            eprintln!("[netease] lyric request failed: {err}");
            return None;
        }
    };

    let body = response.text().unwrap_or_default();
    let parsed: LyricResp = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("[netease] lyric decode failed ({err})");
            return None;
        }
    };

    let lyric = parsed.lrc.and_then(|l| l.lyric)?;
    if lyric.trim().is_empty() || !lyric.contains('[') {
        eprintln!("[netease] empty or unsynced lyric");
        return None;
    }
    Some(lyric)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(name: &str, artist: &str, seconds: f64) -> Song {
        Song {
            id: 1,
            name: name.into(),
            artists: vec![Artist { name: artist.into() }],
            duration: seconds * 1000.0,
        }
    }

    fn query(title: &str, artist: &str, seconds: f64) -> Query {
        Query {
            title: title.into(),
            artist: artist.into(),
            album: String::new(),
            duration_secs: seconds,
            app_id: "test".into(),
        }
    }

    #[test]
    fn rejects_cover_by_artist_and_duration() {
        let q = query("不能说的秘密", "周杰伦", 296.5);
        let cover = song("不能说的秘密 (女声版)", "余不不", 295.0);
        assert!(score(&cover, &q).is_none(), "a cover must not win on duration alone");
    }

    #[test]
    fn accepts_exact_track() {
        let q = query("不能说的秘密", "周杰伦", 296.5);
        let real = song("不能说的秘密", "周杰伦", 297.0);
        assert!(score(&real, &q).is_some());
    }

    #[test]
    fn tolerates_slightly_wrong_duration_when_artist_matches() {
        let q = query("不能说的秘密", "周杰伦", 296.5);
        let live = song("不能说的秘密", "周杰伦", 376.5);
        assert!(score(&live, &q).is_some());
    }

    #[test]
    fn rejects_unrelated_title() {
        let q = query("晴天", "周杰伦", 269.0);
        let other = song("不能说的秘密", "周杰伦", 297.0);
        assert!(score(&other, &q).is_none());
    }
}
