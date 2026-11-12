import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./styles.css";

// Surfaced through the debug DevTools port so startup failures are visible
// instead of silently leaving an empty overlay.
declare global {
  interface Window {
    __lf_error?: string;
    __lf_snapshots?: number;
    __lf_lyric_sets?: number;
    /** Debug hook: lets a CDP session compare our clock against the player's. */
    __lf_state?: () => {
      pos: number;
      idx: number;
      lines: number;
      cur: string;
      playing: string;
    };
  }
}
window.addEventListener("error", (e) => {
  window.__lf_error = `${e.message} @ ${e.filename}:${e.lineno}`;
});
window.addEventListener("unhandledrejection", (e) => {
  window.__lf_error = String((e.reason as Error)?.message ?? e.reason);
});

/** ---------- backend contract (see src-tauri/src/provider/mod.rs) ---------- */

type PlaybackState = "playing" | "paused" | "stopped" | "unknown";

interface TrackInfo {
  title: string;
  artist: string;
  album: string;
  duration_secs: number;
  app_id: string;
}

interface Snapshot {
  track: TrackInfo | null;
  state: PlaybackState;
  position_secs: number;
  duration_secs: number;
  report_age_ms: number;
  source: string;
}

interface WordToken {
  start: number;
  end: number;
  text: string;
}

interface LyricLine {
  start: number;
  end: number;
  text: string;
  /** empty when the source is line-level only (LRC) */
  words: WordToken[];
}

interface LyricSet {
  level: "word" | "line" | "none";
  lines: LyricLine[];
  offset_ms: number;
  origin: string;
}

/** ---------------------------- local clock ---------------------------- */

/** Position is published every few hundred ms; between reports we run our own
 *  clock so the highlight moves at 60fps instead of stepping.
 *
 *  The CDP feed is worse than "every few hundred ms": the player's internal
 *  `_currentTime` is quantised to whole seconds (measured: it sits on 807.757 for
 *  ~1.1s, then jumps to 808.757). Re-anchoring on every snapshot therefore drags
 *  the display backwards by up to a second, which reads as stuttering. So we keep
 *  our own monotonic clock and only bleed a fraction of the error in each time. */
class PlayClock {
  private shown = 0;
  private at = 0;
  private rate = 0;

  sync(position: number, state: PlaybackState): void {
    const now = performance.now();
    const playing = state === "playing";

    if (!playing) {
      this.shown = position;
      this.at = now;
      this.rate = 0;
      return;
    }

    const expected = this.shown + (now - this.at) / 1000;
    const err = position - expected;

    // A big error is a seek or a new track: believe the source and jump.
    if (Math.abs(err) > 2) {
      this.shown = position;
      this.at = now;
      this.rate = 1;
      return;
    }

    const correction = Math.max(-0.12, Math.min(0.12, err)) * 0.35;
    this.shown = Math.max(this.shown, expected + correction);
    this.at = now;
    this.rate = 1;
  }

  now(offsetMs: number): number {
    const elapsed = this.rate === 0 ? 0 : (performance.now() - this.at) / 1000;
    return this.shown + elapsed + offsetMs / 1000;
  }
}

/** ------------------------------ state ------------------------------ */

const stage = document.getElementById("stage") as HTMLDivElement;
const elCurrent = document.getElementById("line-current") as HTMLDivElement;
const elNext = document.getElementById("line-next") as HTMLDivElement;
const elHint = document.getElementById("hint") as HTMLDivElement;

const clock = new PlayClock();
let lyrics: LyricSet = { level: "none", lines: [], offset_ms: 0, origin: "" };
let activeIndex = -1;
let editing = false;
let hintTimer = 0;
let currentTrack: TrackInfo | null = null;
let placeholderShown = "";

function applyMode(): void {
  const playing = clockState === "playing";
  const hasSomething = lyrics.lines.length > 0 || currentTrack !== null;
  stage.dataset.mode = editing ? "edit" : hasSomething ? "live" : "idle";
  stage.style.setProperty("--opacity", playing || editing ? "" : "0.55");
}

let clockState: PlaybackState = "unknown";

function showHint(text: string): void {
  elHint.textContent = text;
  elHint.classList.add("show");
  window.clearTimeout(hintTimer);
  hintTimer = window.setTimeout(() => elHint.classList.remove("show"), 1400);
}

/** ---------------------------- rendering ---------------------------- */

function buildLine(line: LyricLine): void {
  elCurrent.textContent = "";
  elCurrent.dataset.level = line.words.length ? "word" : "line";

  const tokens: { text: string; start: number; end: number }[] = line.words.length
    ? line.words.map((w) => ({ text: w.text, start: w.start, end: w.end }))
    : [{ text: line.text, start: line.start, end: line.end }];

  const stack = document.createElement("div");
  stack.className = "stack";

  // Bottom layer: the outline. Top layer: the gradient fill.
  const stroke = document.createElement("span");
  stroke.className = "layer stroke";
  const fill = document.createElement("span");
  fill.className = "layer fill";

  for (const token of tokens) {
    const outline = document.createElement("span");
    outline.className = "w";
    outline.textContent = token.text;
    stroke.appendChild(outline);

    const word = document.createElement("span");
    word.className = "w";
    word.textContent = token.text;
    word.dataset.start = String(token.start);
    word.dataset.end = String(token.end);
    fill.appendChild(word);
  }

  stack.append(stroke, fill);
  elCurrent.appendChild(stack);
}

function paintWords(t: number): void {
  const fill = elCurrent.querySelector(".layer.fill");
  if (!fill) return;
  const spans = fill.children;

  for (let i = 0; i < spans.length; i++) {
    const el = spans[i] as HTMLSpanElement;
    if (elCurrent.dataset.level === "line") {
      el.dataset.lit = "1";
      continue;
    }
    const start = Number(el.dataset.start ?? 0);
    const end = Number(el.dataset.end ?? 0);
    let p: number;
    if (t <= start) {
      p = 0;
    } else if (t >= end) {
      p = 1;
    } else {
      p = (t - start) / Math.max(0.001, end - start);
    }
    el.style.setProperty("--p", p.toFixed(4));
  }
}

function tick(): void {
  requestAnimationFrame(tick);

  if (lyrics.lines.length === 0) {
    // No lyrics for this track: still show what is playing, so the overlay is
    // never a mysterious invisible rectangle.
    if (currentTrack) {
      const text = `${currentTrack.title} — ${currentTrack.artist}`;
      if (placeholderShown !== text) {
        placeholderShown = text;
        activeIndex = -1;
        buildLine({ start: 0, end: 0, text, words: [] });
        elNext.textContent = "歌词获取中 / 未找到";
        const fill = elCurrent.querySelector(".layer.fill");
        fill?.querySelectorAll(".w").forEach((node) => {
          (node as HTMLElement).dataset.lit = "1";
        });
      }
    }
    return;
  }

  const t = clock.now(lyrics.offset_ms);

  // Binary-search-free scan is fine for a few hundred lines.
  let idx = -1;
  for (let i = 0; i < lyrics.lines.length; i++) {
    const line = lyrics.lines[i];
    if (t >= line.start && t < line.end) {
      idx = i;
      break;
    }
  }
  if (idx === -1 && t >= lyrics.lines[lyrics.lines.length - 1].end) {
    idx = lyrics.lines.length - 1;
  }

  // Paused before the first line, or sitting in an instrumental gap: show where we are
  // heading instead of leaving the window completely blank.
  if (idx === -1) {
    const upcoming = lyrics.lines.find((line) => line.start > t);
    const text = currentTrack ? `${currentTrack.title} — ${currentTrack.artist}` : "LyricFloat";
    if (placeholderShown !== text) {
      placeholderShown = text;
      buildLine({ start: 0, end: 0, text, words: [] });
      const fill = elCurrent.querySelector(".layer.fill");
      fill?.querySelectorAll(".w").forEach((node) => {
        (node as HTMLElement).dataset.lit = "1";
      });
    }
    elNext.textContent = upcoming ? upcoming.text : "";
    activeIndex = -1;
    return;
  }

  if (idx !== activeIndex) {
    activeIndex = idx;
    if (idx >= 0) buildLine(lyrics.lines[idx]);
    const next = lyrics.lines[idx + 1];
    elNext.textContent = next ? next.text : "";
  }
  if (activeIndex >= 0) paintWords(t);
}

requestAnimationFrame(tick);

window.__lf_state = () => ({
  pos: Number(clock.now(lyrics.offset_ms).toFixed(3)),
  idx: activeIndex,
  lines: lyrics.lines.length,
  cur: elCurrent.textContent ?? "",
  playing: clockState,
});

/** ------------------------------ wiring ------------------------------ */

void listen<Snapshot>("player://snapshot", (event) => {
  const snap = event.payload;
  window.__lf_snapshots = (window.__lf_snapshots ?? 0) + 1;
  clockState = snap.state;
  clock.sync(snap.position_secs, snap.state);
  if (snap.track?.title !== currentTrack?.title) placeholderShown = "";
  currentTrack = snap.track;
  applyMode();
});

function applyLyricSet(set: LyricSet): void {
  lyrics = set;
  activeIndex = -1;
  // The DOM is cleared below; let the placeholder repaint itself.
  placeholderShown = "";
  elCurrent.textContent = "";
  elNext.textContent = "";
  applyMode();
  if (lyrics.lines.length) {
    showHint(`${lyrics.origin} · ${lyrics.level === "word" ? "逐字" : "逐行"}`);
  } else {
    showHint("未找到歌词");
  }
}

void listen<LyricSet>("lyrics://set", (event) => {
  window.__lf_lyric_sets = (window.__lf_lyric_sets ?? 0) + 1;
  applyLyricSet(event.payload);
});

// A page reload (HMR, or the user reopening the window) must not lose the
// lyrics the backend already has: pull the cached set once on startup.
void invoke<LyricSet | null>("get_lyrics")
  .then((cached) => {
    if (cached && cached.lines.length) applyLyricSet(cached);
  })
  .catch(() => {
    /* backend may not be ready yet; the next track change will push anyway */
  });

void listen<string>("overlay://hint", (event) => showHint(event.payload));

// Nothing visible until a track shows up, so prove we are alive on start.
showHint("LyricFloat 就绪 · 等待播放器");

void listen<boolean>("overlay://edit-mode", (event) => {
  editing = event.payload;
  applyMode();
  showHint(editing ? "方向键调整位置 · 5 秒后自动退出" : "已锁定穿透");
});

// Drag the window while in edit mode.
let dragging = false;
let dragOrigin = { x: 0, y: 0 };

stage.addEventListener("mousedown", (e) => {
  if (!editing || e.button !== 0) return;
  dragging = true;
  dragOrigin = { x: e.clientX, y: e.clientY };
  void invoke("start_drag");
});

window.addEventListener("mousemove", (e) => {
  if (!dragging) return;
  void invoke("drag_to", {
    dx: e.clientX - dragOrigin.x,
    dy: e.clientY - dragOrigin.y,
  });
});

window.addEventListener("mouseup", () => {
  if (!dragging) return;
  dragging = false;
  void invoke("end_drag");
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && editing) void invoke("set_edit_mode", { enabled: false });
});
