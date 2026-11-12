// Reads the live cookie jar of the debugged Chrome through CDP.
// Used to answer "does the dedicated profile keep the login?" with real expiry dates
// instead of guessing (the SQLite file itself is exclusively locked while Chrome runs).
//
// Usage: node tools/cdp-cookies.mjs [port] [domainFilter]

const port = process.argv[2] ?? "9222";
const filter = process.argv[3] ?? "apple.com";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.log("no page target");
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
const send = (id, method, params) => ws.send(JSON.stringify({ id, method, params }));

ws.addEventListener("open", () => {
  send(1, "Network.enable");
  send(2, "Network.getCookies", { urls: [`https://music.apple.com/`, `https://amp-api.music.apple.com/`] });
});

ws.addEventListener("message", (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id !== 2) return;

  const cookies = msg.result?.cookies ?? [];
  const wanted = cookies.filter((c) => c.domain.includes(filter));
  console.log(`cookies for *${filter}*: ${wanted.length}`);
  const rows = wanted
    .map((c) => ({
      name: c.name,
      domain: c.domain,
      expires: c.expires > 0 ? new Date(c.expires * 1000).toISOString().slice(0, 16).replace("T", " ") : "session-only",
      days: c.expires > 0 ? Math.round((c.expires * 1000 - Date.now()) / 86400000) : null,
    }))
    .sort((a, b) => (b.days ?? -1) - (a.days ?? -1));
  for (const r of rows) {
    console.log(`  ${r.name.padEnd(28)} ${r.domain.padEnd(24)} ${r.expires}${r.days !== null ? `  (${r.days}d)` : ""}`);
  }
  ws.close();
  process.exit(0);
});

setTimeout(() => process.exit(0), 8000);
