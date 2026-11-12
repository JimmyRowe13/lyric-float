// LyricFloat Bridge — isolated world. Owns the local WebSocket.
//
// Kept out of the page world on purpose: this script runs under the extension's
// security context, so music.apple.com's CSP cannot block ws://127.0.0.1 here.
//
// Change the port/token here and in the overlay's %APPDATA%\lyric-float\config.json.

(() => {
  const PORT = 8787;
  const TOKEN = "lyricfloat";
  const RETRY_MS = 3000;

  if (window.__lfBridgeInstalled) return;
  window.__lfBridgeInstalled = true;

  let socket = null;
  let connected = false;

  function connect() {
    try {
      socket = new WebSocket("ws://127.0.0.1:" + PORT + "/" + TOKEN);
    } catch (e) {
      setTimeout(connect, RETRY_MS);
      return;
    }
    socket.addEventListener("open", () => {
      connected = true;
      console.info("[LyricFloat] overlay connected");
      // Tell the page world to re-send lyrics: the overlay may have just restarted and
      // has nothing cached.
      try {
        window.postMessage({ __lfBridgeConnected: true }, "*");
      } catch (e) { /* ignore */ }
    });
    socket.addEventListener("close", () => {
      connected = false;
      socket = null;
      setTimeout(connect, RETRY_MS);
    });
    socket.addEventListener("error", () => {
      try { socket && socket.close(); } catch (e) { /* ignore */ }
    });
  }

  window.addEventListener("message", (event) => {
    if (event.source !== window) return;
    const data = event.data;
    if (!data || !data.__lfBridge || !data.msg) return;
    if (!connected || !socket) return;
    try {
      socket.send(JSON.stringify(data.msg));
    } catch (e) { /* ignore */ }
  });

  connect();
  console.info("[LyricFloat] transport bridge installed");
})();
