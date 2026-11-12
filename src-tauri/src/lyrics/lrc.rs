//! LRC parser: line-level timestamps, multi-timestamp lines, `[offset:]` tag.
//!
//! This is the tier-2 format, so the parser is deliberately forgiving: real-world
//! LRC files are hand-edited and full of `[mm:ss]` without fractions, full-width
//! brackets, and blank lines carrying a timestamp.

use super::LyricLine;

/// Extracts the `[offset:+/-ms]` global tag, defaulting to 0.
pub fn offset_ms(text: &str) -> i64 {
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("[offset:") {
            if let Some(num) = rest.split(']').next() {
                if let Ok(v) = num.trim().parse::<i64>() {
                    return v;
                }
            }
        }
    }
    0
}

fn parse_stamp(stamp: &str) -> Option<f64> {
    // Accept mm:ss, mm:ss.x, mm:ss.xxx, hh:mm:ss.xxx
    let parts: Vec<&str> = stamp.split(':').collect();
    let (h, m, s) = match parts.len() {
        2 => (0.0, parts[0].parse::<f64>().ok()?, parts[1]),
        3 => (
            parts[0].parse::<f64>().ok()?,
            parts[1].parse::<f64>().ok()?,
            parts[2],
        ),
        _ => return None,
    };
    let sec: f64 = s.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

fn stamps(line: &str) -> (Vec<f64>, &str) {
    let mut out = Vec::new();
    let mut rest = line;
    loop {
        let t = rest.trim_start();
        if !t.starts_with('[') {
            return (out, t);
        }
        let Some(close) = t.find(']') else { return (out, t) };
        let inner = &t[1..close];
        match parse_stamp(inner.trim()) {
            Some(v) => {
                out.push(v);
                rest = &t[close + 1..];
            }
            // A non-timestamp tag like [ar:...] or [ti:...]
            None => return (out, t[close + 1..].trim_start()),
        }
    }
}

pub fn parse(text: &str) -> Vec<LyricLine> {
    let mut lines: Vec<LyricLine> = Vec::new();

    for raw in text.lines() {
        let raw = raw.trim_start_matches('\u{feff}');
        let (times, content) = stamps(raw);
        if times.is_empty() {
            continue;
        }
        let content = content.trim().to_string();
        // Keep instrumental gaps (empty content) so the renderer can blank the line.
        for start in times {
            lines.push(LyricLine { start, end: 0.0, text: content.clone(), words: Vec::new() });
        }
    }

    lines.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));

    // End of each line = start of the next (or start + 5s for the last one).
    for i in 0..lines.len() {
        let next = lines.get(i + 1).map(|l| l.start);
        lines[i].end = next.unwrap_or(lines[i].start + 5.0);
    }
    lines
}

/// Shift every timestamp by `delta_ms`, keeping order.
pub fn apply_offset(lines: &mut [LyricLine], delta_ms: i64) {
    let d = delta_ms as f64 / 1000.0;
    for line in lines.iter_mut() {
        line.start = (line.start + d).max(0.0);
        line.end = (line.end + d).max(0.0);
        for w in line.words.iter_mut() {
            w.start = (w.start + d).max(0.0);
            w.end = (w.end + d).max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_and_computes_ends() {
        let text = "[ti:Test]\n[00:12.50]first\n[00:15.00]second\n";
        let lines = parse(text);
        assert_eq!(lines.len(), 2);
        assert!((lines[0].start - 12.5).abs() < 1e-6);
        assert_eq!(lines[0].text, "first");
        assert!((lines[0].end - 15.0).abs() < 1e-6);
        assert!((lines[1].end - 20.0).abs() < 1e-6);
    }

    #[test]
    fn handles_multiple_timestamps_on_one_line() {
        let lines = parse("[00:01.00][00:05.00]chorus\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].start, 5.0);
        assert_eq!(lines[1].text, "chorus");
    }

    #[test]
    fn handles_hours_and_no_fraction() {
        let lines = parse("[1:02:03]long\n");
        assert_eq!(lines.len(), 1);
        assert!((lines[0].start - 3723.0).abs() < 1e-6);
    }

    #[test]
    fn reads_offset_tag() {
        assert_eq!(offset_ms("[offset:-250]\n[00:01]x"), -250);
        assert_eq!(offset_ms("[00:01]x"), 0);
    }

    #[test]
    fn keeps_empty_instrumental_lines() {
        let lines = parse("[00:01]sing\n[00:10]\n[00:20]sing again\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].text, "");
    }

    #[test]
    fn sorts_out_of_order_input() {
        let lines = parse("[00:20]b\n[00:10]a\n");
        assert_eq!(lines[0].text, "a");
        assert_eq!(lines[1].text, "b");
    }

    #[test]
    fn offset_shifts_and_clamps() {
        let mut lines = parse("[00:01.00]x\n");
        apply_offset(&mut lines, -2000);
        assert_eq!(lines[0].start, 0.0);
    }
}
