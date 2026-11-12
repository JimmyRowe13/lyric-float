// Records the lyrics-related network traffic of a Chrome page over CDP.
// Purpose: find out exactly which URL/headers Apple Music's own player uses for
// lyrics, and capture the real payload (expected to be TTML).
//
// Usage: node tools/cdp-netlog.mjs <out.jsonl> [port] [durationSeconds]

import { appendFileSync, writeFileSync } from "node:fs";

const out = process.argv[2] ?? "netlog.jsonl";
const port = process.argv[3] ?? "9222";
const duration = Number(process.argv[4] ?? 180);

const interesting = (url) =>
  /lyric|ttml/i.test(url) || /amp-api\.music\.apple\.com/.test(url);

// Console only shows the payload we actually care about; the JSONL keeps everything.
const loud = (url) => /lyric|ttml/i.test(url);

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page" && /music\.apple\.com/.test(t.url)) ??
  targets.find((t) => t.type === "page");
if (!page) {
  console.log("no page target");
  process.exit(1);
}
console.log("attached to:", page.url);
console.log("writing to:", out, "for", duration, "s");

writeFileSync(out, "");
const record = (obj) => appendFileSync(out, JSON.stringify(obj) + "\n");

const ws = new WebSocket(page.webSocketDebuggerUrl);
const pending = new Map();
let nextId = 1;

const send = (method, params = {}) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, resolve);
    ws.send(JSON.stringify({ id, method, params }));
  });

ws.addEventListener("open", async () => {
  await send("Network.enable");
  await send("Page.enable");
  console.log("network domain enabled");
});

ws.addEventListener("message", async (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg.result);
    pending.delete(msg.id);
    return;
  }

  if (msg.method === "Network.requestWillBeSent") {
    const { requestId, request } = msg.params;
    if (interesting(request.url)) {
      record({ kind: "request", requestId, url: request.url, method: request.method, headers: request.headers });
      if (loud(request.url)) console.log("REQ", request.method, request.url.slice(0, 200));
    }
    return;
  }

  if (msg.method === "Network.responseReceived") {
    const { requestId, response } = msg.params;
    if (!interesting(response.url)) return;
    record({
      kind: "response",
      requestId,
      url: response.url,
      status: response.status,
      mimeType: response.mimeType,
      headers: response.headers,
    });
    if (loud(response.url)) console.log("RES", response.status, response.mimeType, response.url.slice(0, 200));
    try {
      const body = await send("Network.getResponseBody", { requestId });
      if (body && body.body) {
        record({
          kind: "body",
          requestId,
          url: response.url,
          base64: !!body.base64Encoded,
          length: body.body.length,
          body: body.body.slice(0, 200_000),
        });
        if (loud(response.url)) console.log("BODY", body.body.length, "chars");
      }
    } catch (e) {
      record({ kind: "body-error", requestId, url: response.url, error: String(e) });
    }
  }
});

setTimeout(() => {
  console.log("done");
  try {
    ws.close();
  } catch {}
  process.exit(0);
}, duration * 1000);
