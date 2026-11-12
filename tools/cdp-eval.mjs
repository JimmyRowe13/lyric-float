// Runs one JS expression inside the overlay page over the WebView2 DevTools port.
// Passing the expression via a file avoids PowerShell quoting hell.
//
// Usage: node tools/cdp-eval.mjs <expression-file> [port]

import { readFileSync } from "node:fs";

const file = process.argv[2];
const port = process.argv[3] ?? "9333";
const timeoutMs = Number(process.argv[4] ?? 6000);
if (!file) {
  console.error("usage: node cdp-eval.mjs <expression-file> [port]");
  process.exit(2);
}
const expression = readFileSync(file, "utf8");

const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.log("no page target");
  process.exit(1);
}

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
  if (msg.result?.exceptionDetails) {
    console.log("EXCEPTION:", JSON.stringify(msg.result.exceptionDetails).slice(0, 600));
  } else {
    const v = msg.result?.result?.value;
    console.log(typeof v === "string" ? v : JSON.stringify(v, null, 2));
  }
  ws.close();
  process.exit(0);
});
setTimeout(() => process.exit(0), timeoutMs);
