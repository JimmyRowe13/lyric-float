// Simulates the LyricFloat Bridge extension: connects to the overlay's local socket,
// sends one playback state and one TTML payload. Used to verify the extension feed
// without loading the real extension into Chrome.
//
// Usage: node tools/simulate-extension.mjs [port] [token] [ttmlPath]

import { readFileSync } from "node:fs";

const port = process.argv[2] ?? "8787";
const token = process.argv[3] ?? "lyricfloat";
const ttmlPath = process.argv[4] ?? "";

const ws = new WebSocket(`ws://127.0.0.1:${port}/${token}`);

ws.addEventListener("open", () => {
  console.log("connected");
  const state = {
    type: "state",
    payload: {
      playing: true,
      position: 42.5,
      duration: 196.6,
      title: "连通性测试",
      artist: "LyricFloat",
      album: "probe",
      songId: "probe-1",
      storefront: "cn",
    },
  };
  ws.send(JSON.stringify(state));
  console.log("sent state");

  if (ttmlPath) {
    try {
      const ttml = readFileSync(ttmlPath, "utf8");
      ws.send(JSON.stringify({ type: "lyrics", songId: "probe-1", ttml }));
      console.log("sent lyrics", ttml.length, "chars");
    } catch (e) {
      console.log("cannot read ttml:", e.message);
    }
  }
});

ws.addEventListener("error", (e) => console.log("ws error", e.message));
setTimeout(() => {
  try { ws.close(); } catch {}
  process.exit(0);
}, 6000);
