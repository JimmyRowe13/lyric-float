# M2 — 接真实播放会话的记录

日期：2026-11-10 22:2x–22:4x (UTC+8)
播放源：网页版 Apple Music（Chrome 128），曲目：周杰伦《不能说的秘密》，376.458s

## 结论

| 项 | 结果 |
|---|---|
| 浮窗在桌面上真实可见 | ✅ 文字浮在 Chrome 之上，双行结构正常 |
| SMTC 拿到曲名/歌手/曲长 | ✅ |
| SMTC 拿到"实时"进度 | ❌ **只在事件点上报一次** |
| 歌词自动获取 | ❌ 三层源全部落空（见下） |

## 1. SMTC 的进度是"一次性快照"，不是实时流

45 秒连续采样，`pos` 一直是 `15.873`、`lastUpdated` 一次没刷新：

```
i=0  pos=15.873  age=10.9s
i=8  pos=15.873  age=27.0s
i=17 pos=15.873  age=45.1s
```

对照实验（自建页面每 250ms 调 `setPositionState`）里进度是连续刷新的，说明**不是 Windows 的
限制，是 Apple Music 网页版不持续上报**。

因此策略改为：把 `LastUpdatedTime` 当作锚点，播放中一律外推
`position = pos + (now - lastUpdated) × rate`。这是 `provider/smtc.rs` 里现在的行为
（`EXTRAPOLATE_AFTER_MS = 0`）。暂停/跳转时 Chrome 会重新上报一次，锚点自动更新。

外推的代价：整首歌靠一个锚点计时，音频时钟与系统时钟的微小偏差会累积。逐行歌词够用，
逐字仍然需要 CDP 的 `audio.currentTime`。

## 2. 浮窗"看不见"的两个真 bug（都不是渲染问题）

### 2.1 Tauri 2 默认不给前端任何权限 —— `event.listen` 被拒

现象：窗口存在、WebView2 子进程正常、vite 也连上了，但屏幕上永远空白。

用 WebView2 的调试端口（`--remote-debugging-port=9333`，debug 构建才有）进页面求值才发现：

```
err: "event.listen not allowed. Permissions associated with this command:
      core:event:allow-listen, core:event:default"
```

没有 capabilities 文件 → `listen()` 全部失败 → 一个事件都收不到 → 永远 idle（opacity 0）。

修复：新增 `src-tauri/capabilities/default.json`，授予 `core:default`、`core:event:default`、
`core:window:default`、`core:app:default`。

### 2.2 占位文本清空后不会重画

收到空歌词时把 `#line-current` 清空，但 `placeholderShown` 缓存没重置，`tick()` 认为
"已经画过了"就跳过，于是永远空白。修复：在 `lyrics://set` 和曲目变化时把 `placeholderShown`
置空。

## 3. 歌词源调查（这是当前的真正阻塞）

| 源 | 结果 |
|---|---|
| 网易云 `music.163.com/api/search/get/web` | ✅ 接口可用，❌ **没有周杰伦版权**（20 条结果全是翻唱） |
| QQ 音乐 `u.y.qq.com/cgi-bin/musicu.fcg` | ❌ `code 2001`，未签名请求一律拒绝 |
| QQ 音乐旧接口 `c.y.qq.com/soso/...` | ❌ 404，已废弃 |
| lrclib.net | ⚠️ 接口本身正常（能查到 `不能说的秘密 / 周杰伦 / 296.53s`），但从本机**间歇性连不通**：一次 200 OK、下一次 TCP connect timeout。rustls 拿到 200 后 body 流被掐断；换 native-tls 直接连不上。属于网络/链路问题，不是代码问题 |

结论：**对 Apple Music 的内容，唯一可靠的歌词来源是 Apple 自己那份 TTML**。它和播放器同源、
逐字节对得上时间轴，并且是逐字级别。拿它的唯一干净路径是 CDP。

## 4. 下一步：CDP provider

1. `tools/start-chrome-cdp.ps1` 用独立 profile + `--remote-debugging-port` 拉起 Chrome 并打开
   `music.apple.com`（已提供）。
2. `provider/cdp.rs`：连 `ws://127.0.0.1:9222/devtools/page/<id>`，
   `Runtime.evaluate` 读 `document.querySelector('audio').currentTime` 拿毫秒级进度；
   `Network.enable` 后截 `amp-api.music.apple.com` 的歌词响应拿 TTML。
3. TTML 解析器已经写好并有单测（`lyrics/ttml.rs`，4 个用例含中文逐字），只等喂真实样本。

## 调试工具（已随仓库）

| 脚本 | 用途 |
|---|---|
| `tools/inspect-webview.mjs` | 读浮窗页面的 DOM 状态（mode / 文本 / 快照计数 / JS 错误） |
| `tools/screenshot-webview.mjs` | 让页面自己截图，区分"页面没画"和"窗口没合成" |
| `tools/cdp-eval.mjs` | 在页面里执行任意表达式（从文件读，避开引号地狱） |
