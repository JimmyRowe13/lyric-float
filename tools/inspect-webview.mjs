// Dumps the overlay's DOM state over the WebView2 DevTools port.
// Debug builds of LyricFloat open 127.0.0.1:9333; this is the fastest way to see
// whether the page received snapshots/lyrics without staring at a transparent window.
//
// Usage: node tools/inspect-webview.mjs [port]

const port = process.argv[2] ?? "9333";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.log("no page target; targets =", targets.map((t) => t.type));
  process.exit(1);
}
console.log("target:", page.url);

const expression = `JSON.stringify({
  mode: document.getElementById('stage')?.dataset.mode,
  cur: document.getElementById('line-current')?.textContent,
  next: document.getElementById('line-next')?.textContent,
  hint: document.getElementById('hint')?.textContent,
  snaps: window.__lf_snapshots ?? 0,
  lyricSets: window.__lf_lyric_sets ?? 0,
  err: window.__lf_error ?? null,
  opacity: getComputedStyle(document.getElementById('stage')).opacity,
  hasWebviewApi: typeof window.__TAURI_INTERNALS__ !== 'undefined'
})`;

const ws = new WebSocket(page.webSocketDebuggerUrl);
ws.addEventListener("open", () => {
  ws.send(
    JSON.stringify({
      id: 1,
      method: "Runtime.evaluate",
      params: { expression, returnByValue: true, awaitPromise: true },
    }),
  );
});
ws.addEventListener("message", (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id !== 1) return;
  const value = msg.result?.result?.value;
  if (value) {
    console.log(JSON.stringify(JSON.parse(value), null, 2));
  } else {
    console.log(JSON.stringify(msg, null, 2));
  }
  ws.close();
});
setTimeout(() => {
  try {
    ws.close();
  } catch {}
  process.exit(0);
}, 6000);
