// Captures what the overlay page itself renders, via the WebView2 DevTools port.
// Separates "the page draws nothing" from "the window does not composite".
//
// Usage: node tools/screenshot-webview.mjs [out.png] [port]

import { writeFileSync } from "node:fs";

const out = process.argv[2] ?? "webview.png";
const port = process.argv[3] ?? "9333";

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.log("no page target");
  process.exit(1);
}

const ws = new WebSocket(page.webSocketDebuggerUrl);
ws.addEventListener("open", () => {
  ws.send(JSON.stringify({ id: 1, method: "Page.enable" }));
  ws.send(
    JSON.stringify({
      id: 2,
      method: "Page.captureScreenshot",
      params: { format: "png", fromSurface: true, captureBeyondViewport: false },
    }),
  );
});
ws.addEventListener("message", (event) => {
  const msg = JSON.parse(event.data);
  if (msg.id !== 2) return;
  const data = msg.result?.data;
  if (!data) {
    console.log("capture failed:", JSON.stringify(msg).slice(0, 300));
    process.exit(1);
  }
  writeFileSync(out, Buffer.from(data, "base64"));
  console.log("wrote", out);
  ws.close();
  process.exit(0);
});
setTimeout(() => process.exit(0), 8000);
