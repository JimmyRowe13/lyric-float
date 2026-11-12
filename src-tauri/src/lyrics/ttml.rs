//! Apple Music TTML parser (tier 1, word-level).
//!
//! Apple's lyrics payload is TTML: `<p>` elements are display lines, nested `<span>`
//! elements carry their own `begin`/`end`, and the leaves are syllables or words.
//! `ttm:agent="v1"` is the lead vocal, `v2` background — we keep `v1` by default.
//!
//! Parsing rules that matter in practice:
//!   * whitespace-only text between two spans is formatting, not content;
//!   * a `<p>` without child spans still has to become a line (and a single token);
//!   * a span without its own timing inherits the parent's.

use super::{LyricLine, WordToken};
use quick_xml::events::Event;
use quick_xml::Reader;

fn parse_time(raw: &str) -> Option<f64> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    if let Some(ms) = v.strip_suffix("ms") {
        return ms.parse::<f64>().ok().map(|n| n / 1000.0);
    }
    if let Some(secs) = v.strip_suffix('s') {
        return secs.parse::<f64>().ok();
    }
    let parts: Vec<&str> = v.split(':').collect();
    match parts.len() {
        3 => Some(
            parts[0].parse::<f64>().ok()? * 3600.0
                + parts[1].parse::<f64>().ok()? * 60.0
                + parts[2].parse::<f64>().ok()?,
        ),
        2 => Some(parts[0].parse::<f64>().ok()? * 60.0 + parts[1].parse::<f64>().ok()?),
        1 => v.parse::<f64>().ok(),
        _ => None,
    }
}

struct OpenSpan {
    begin: Option<f64>,
    end: Option<f64>,
    text: String,
    children: usize,
}

pub fn parse(xml: &str, keep_agents: &[&str]) -> Vec<LyricLine> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut lines: Vec<LyricLine> = Vec::new();

    let mut in_para = false;
    let mut para_start = 0.0f64;
    let mut para_end = 0.0f64;
    let mut para_words: Vec<WordToken> = Vec::new();
    let mut para_text = String::new();
    let mut stack: Vec<OpenSpan> = Vec::new();
    // Whitespace *between* two spans is a real separator in Latin lyrics, but must
    // not be invented for CJK lyrics (which are split per character). So we record
    // it only when the source actually has it.
    let mut pending_space = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let local = String::from_utf8_lossy(name.local_name().as_ref()).to_string();
                if local == "p" || local == "div" {
                    if local == "p" {
                        let mut begin = None;
                        let mut end = None;
                        let mut agent_ok = keep_agents.is_empty();
                        for attr in e.attributes().flatten() {
                            let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                            let val = attr.unescape_value().unwrap_or_default().to_string();
                            match key.as_str() {
                                "begin" => begin = parse_time(&val),
                                "end" => end = parse_time(&val),
                                "ttm:agent" | "agent" => {
                                    if keep_agents.iter().any(|a| *a == val) {
                                        agent_ok = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !agent_ok {
                            // Skip background-vocal paragraphs entirely.
                            continue;
                        }
                        in_para = true;
                        para_start = begin.unwrap_or(0.0);
                        para_end = end.unwrap_or(0.0);
                        para_words.clear();
                        para_text.clear();
                        pending_space = false;
                    }
                    continue;
                }
                if in_para && local == "span" {
                    let mut begin = None;
                    let mut end = None;
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let val = attr.unescape_value().unwrap_or_default().to_string();
                        match key.as_str() {
                            "begin" => begin = parse_time(&val),
                            "end" => end = parse_time(&val),
                            _ => {}
                        }
                    }
                    if let Some(top) = stack.last_mut() {
                        top.children += 1;
                    }
                    stack.push(OpenSpan { begin, end, text: String::new(), children: 0 });
                }
            }
            Ok(Event::Text(t)) => {
                let raw = t.unescape().unwrap_or_default().to_string();
                if raw.trim().is_empty() {
                    // Separator between two spans, not content.
                    if in_para && stack.is_empty() {
                        pending_space = true;
                    }
                    continue; // formatting whitespace between elements
                }
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&raw);
                }
                if in_para {
                    para_text.push_str(&raw);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let local = String::from_utf8_lossy(name.local_name().as_ref()).to_string();
                if local == "p" || local == "div" {
                    if local == "p" && in_para {
                        in_para = false;
                        let mut words = std::mem::take(&mut para_words);
                        if words.is_empty() && !para_text.trim().is_empty() {
                            words.push(WordToken {
                                start: para_start,
                                end: para_end,
                                text: para_text.clone(),
                            });
                        }
                        let text = if words.is_empty() {
                            para_text.trim().to_string()
                        } else {
                            words.iter().map(|w| w.text.as_str()).collect::<String>().trim().to_string()
                        };
                        lines.push(LyricLine {
                            start: para_start,
                            end: para_end,
                            text,
                            words,
                        });
                        para_text.clear();
                        pending_space = false;
                    }
                    continue;
                }
                if local == "span" {
                    if let Some(span) = stack.pop() {
                        // Leaves become tokens; parents already aggregated their children's text.
                        if span.children == 0 && !span.text.trim().is_empty() {
                            let mut text = span.text;
                            if pending_space && !para_words.is_empty() {
                                text.insert(0, ' ');
                            }
                            pending_space = false;
                            para_words.push(WordToken {
                                start: span.begin.unwrap_or(para_start),
                                end: span.end.unwrap_or(para_end),
                                text,
                            });
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    lines.retain(|l| l.end > l.start || !l.words.is_empty());
    lines.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

/// Fallback end times: a paragraph may omit `end`.
pub fn normalize(lines: &mut [LyricLine], tail: f64) {
    for i in 0..lines.len() {
        if lines[i].end <= lines[i].start {
            lines[i].end = lines
                .get(i + 1)
                .map(|l| l.start)
                .unwrap_or(lines[i].start + tail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata">
  <body><div>
    <p begin="00:00:12.000" end="00:00:14.000" ttm:agent="v1">
      <span begin="00:00:12.000" end="00:00:12.800">Hello</span>
      <span begin="00:00:12.800" end="00:00:14.000">world</span>
    </p>
    <p begin="00:00:14.000" end="00:00:16.000" ttm:agent="v1">plain line</p>
    <p begin="00:00:16.000" end="00:00:18.000" ttm:agent="v2">backing</p>
  </div></body>
</tt>"#;

    #[test]
    fn parses_words_and_skips_background() {
        let lines = parse(SAMPLE, &["v1"]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[0].words.len(), 2);
        assert!((lines[0].words[1].start - 12.8).abs() < 1e-6);
        // second paragraph has no child spans -> single token from the paragraph timing
        assert_eq!(lines[1].words.len(), 1);
        assert_eq!(lines[1].text, "plain line");
    }

    #[test]
    fn keeps_everything_when_no_agent_filter() {
        let lines = parse(SAMPLE, &[]);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn does_not_inject_spaces_between_cjk_syllables() {
        // CJK TTML splits per character with no whitespace node in between;
        // joining with spaces would corrupt the line.
        let xml = r#"<tt><body><div>
    <p begin="0s" end="2s" ttm:agent="v1"><span begin="0s" end="0.5s">晚</span><span begin="0.5s" end="1s">风</span><span begin="1s" end="1.5s">吹</span></p>
  </div></body></tt>"#;
        let lines = parse(xml, &["v1"]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "晚风吹");
        assert_eq!(lines[0].words.len(), 3);
    }

    #[test]
    fn parses_apple_line_timing_shape() {
        // Shape verified against a real capture from amp-api (docs/m3-apple-lyrics.md):
        // a <div> wrapper, `itunes:key` on each <p>, `itunes:timing="Line"`, and
        // timestamps that may be `M:SS.mmm` rather than `MM:SS.mmm`.
        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:itunes="http://music.apple.com/lyric-ttml-internal" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" itunes:timing="Line" xml:lang="zh-Hans">
  <body dur="3:16.587"><div begin="40.312" end="59.480" itunes:songPart="Verse">
    <p begin="40.312" end="42.110" itunes:key="L1" ttm:agent="v1">first line</p>
    <p begin="1:03.836" end="1:09.024" itunes:key="L2" ttm:agent="v1">second line</p>
  </div></body></tt>"#;
        let lines = parse(xml, &["v1"]);
        assert_eq!(lines.len(), 2);
        assert!((lines[0].start - 40.312).abs() < 1e-6);
        assert!((lines[1].start - 63.836).abs() < 1e-6);
        assert_eq!(lines[1].text, "second line");
        // No spans: one synthetic token covers the whole line.
        assert_eq!(lines[1].words.len(), 1);
    }

    #[test]
    fn parses_time_shapes() {
        assert_eq!(parse_time("00:01:02.500"), Some(62.5));
        assert_eq!(parse_time("01:02.500"), Some(62.5));
        assert_eq!(parse_time("1500ms"), Some(1.5));
        assert_eq!(parse_time("1.5s"), Some(1.5));
        assert_eq!(parse_time("nope"), None);
    }
}
