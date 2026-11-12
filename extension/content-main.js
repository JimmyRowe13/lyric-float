// LyricFloat Bridge — page world (world: "MAIN").
//
// Reads what the CDP edition reads over DevTools, but from inside the page:
//   * clock:  mediaItemPlayback._currentPlayer._buffer
//             songPos = _currentTime - currentTimestampOffset   (a concert album is ONE stream)
//   * track:  currentPlayback.asset.id / item.attributes / mediaSession
//   * lyrics: /v1/catalog/{sf}/songs/{id}/syllable-lyrics  (NOT /lyrics, which 404s on
//             older paths; /lyrics is kept as a fallback and carries attributes.ttml)
//
// It cannot open the WebSocket itself: a MAIN-world script lives under the page's CSP,
// and music.apple.com's connect-src does not allow ws://127.0.0.1. So everything is
// handed to bridge.js (isolated world) through window.postMessage.

(() => {
  if (window.__lfPageInstalled) return;
  window.__lfPageInstalled = true;

  const tickMs = 200;
  const retryMs = 8000;

  let lyricDoneFor = null;   // song id whose lyrics were delivered successfully
  let lyricTriedAt = 0;      // last attempt, for the retry cooldown
  let busy = false;

  const post = (msg) => {
    try {
      window.postMessage({ __lfBridge: true, msg }, "*");
    } catch (e) { /* ignore */ }
  };

  function readState() {
    const kit = window.MusicKit && window.MusicKit.getInstance ? window.MusicKit.getInstance() : null;
    if (!kit) return null;
    const services = kit._playbackController && kit._playbackController.services;
    const mip = services && services.mediaItemPlayback;
    if (!mip) return null;

    const playback = mip.currentPlayback || null;
    const player = mip._currentPlayer || null;
    const buffer = player && player._buffer ? player._buffer : null;
    const attrs = playback && playback.item && playback.item.attributes ? playback.item.attributes : null;
    const meta = navigator.mediaSession && navigator.mediaSession.metadata;

    let position = null;
    if (buffer && Number.isFinite(buffer._currentTime) && Number.isFinite(buffer.currentTimestampOffset)) {
      position = Math.max(0, buffer._currentTime - buffer.currentTimestampOffset);
    }

    return {
      playing: player ? !player._paused : false,
      position,
      duration: buffer && Number.isFinite(buffer.duration) ? buffer.duration : null,
      title: (attrs && attrs.name) || (meta && meta.title) || null,
      artist: (attrs && attrs.artistName) || (meta && meta.artist) || null,
      album: (attrs && attrs.albumName) || (meta && meta.album) || null,
      songId: playback && playback.asset && playback.asset.id ? String(playback.asset.id) : null,
      storefront: kit.storefrontId || "cn",
    };
  }

  async function getJson(url, headers) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 6000);
    try {
      const res = await fetch(url, { headers, signal: ctrl.signal });
      if (!res.ok) return null;
      return await res.json();
    } catch (e) {
      return null;
    } finally {
      clearTimeout(timer);
    }
  }

  // Returns TTML text or null. Two shapes: syllable-lyrics -> attributes.ttmlLocalizations,
  // plain lyrics -> attributes.ttml.
  async function fetchLyrics(songId, storefront) {
    const kit = window.MusicKit && window.MusicKit.getInstance ? window.MusicKit.getInstance() : null;
    if (!kit || !songId) return null;
    const sf = storefront || "cn";
    const headers = {
      Authorization: "Bearer " + kit.developerToken,
      "media-user-token": kit.musicUserToken || "",
      Origin: "https://music.apple.com",
    };
    const base = "https://amp-api.music.apple.com/v1/catalog/" + sf + "/songs/" + songId;

    const syllable = await getJson(
      base + "/syllable-lyrics?l%5Blyrics%5D=zh-hans-cn&l%5Bscript%5D=zh-Hans&extend=ttmlLocalizations",
      headers,
    );
    const fromSyllable = syllable && syllable.data && syllable.data[0] && syllable.data[0].attributes
      ? syllable.data[0].attributes.ttmlLocalizations
      : null;
    if (typeof fromSyllable === "string" && fromSyllable.length > 32) return fromSyllable;

    const plain = await getJson(base + "/lyrics?extend=ttmlLocalizations", headers);
    const fromPlain = plain && plain.data && plain.data[0] && plain.data[0].attributes
      ? plain.data[0].attributes.ttml
      : null;
    if (typeof fromPlain === "string" && fromPlain.length > 32) return fromPlain;

    return null;
  }

  async function tick() {
    if (busy) return;
    busy = true;
    try {
      const state = readState();
      if (!state || !state.title) return;
      post({ type: "state", payload: state });

      if (!state.songId || state.songId === lyricDoneFor) return;
      // Retry cooldown: a failure must not turn into a request every 200 ms, but it also
      // must not be remembered as success (that used to leave a song lyric-less forever).
      if (Date.now() - lyricTriedAt < retryMs) return;
      lyricTriedAt = Date.now();

      const ttml = await fetchLyrics(state.songId, state.storefront);
      if (ttml) {
        lyricDoneFor = state.songId;
        post({ type: "lyrics", songId: state.songId, ttml });
      }
    } finally {
      busy = false;
    }
  }

  // The overlay restarts or reconnects: it has no lyrics in memory, so allow a resend.
  window.addEventListener("message", (event) => {
    if (event.source !== window) return;
    if (event.data && event.data.__lfBridgeConnected) {
      lyricDoneFor = null;
      lyricTriedAt = 0;
    }
  });

  setInterval(tick, tickMs);
  console.info("[LyricFloat] page bridge installed");
})();
