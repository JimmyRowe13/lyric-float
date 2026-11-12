# M3 — Apple 自家歌词打通

## 事后修正：两个把时序搞错的 bug（已修）

第一次跑通后用户反馈"歌词刷新不对、时序对不上"。查出两个独立问题：

### A. 演唱会专辑是**一整条 HLS 流**，媒体元素的 currentTime 是"演出时间"不是"歌曲时间"

```
audio[6]  seekable=[0, 819.726]   currentTime=554.12   duration=null
```

一首 297 秒的歌，播放位置却到了 554 秒——因为整场演唱会是一条 819 秒的流。

权威时钟在播放器内部：

```js
kit._playbackController.services.mediaItemPlayback._currentPlayer._buffer
  ._currentTime             // 607.322446  演出时间
  .currentTimestampOffset   // 502.322446  本曲在流里的起点
  .duration                 // 297.387     本曲时长
```

`songPos = _currentTime - currentTimestampOffset`。实测 607.322446 - 502.322446 = 105.000，
三秒后 616.32 - 502.32 = 114 → 117，逐秒推进。

### B. 用搜索反查 catalog id 会挑错版本

《不能说的秘密》在 Apple 上有 Live(魔天伦)、Live(2007 巡回)、录音室三个版本，
名字和歌手完全相同，时长只差 0.1 秒。按"标题+歌手+hasLyrics"打分全部打平 → 取到列表第一个
（魔天伦版），而实际播的是 2007 巡回版 → 歌词整首错位。

真正准确的 id 就在播放器里：`mediaItemPlayback.currentPlayback.asset.id`
（实测 `536743627`，正是 2007 巡回版）。现在直接用它，搜索只作为兜底。

### C. 播放器时钟是 1 秒粒度，每 200ms 重新对锚就会"鬼畜"

修完 A/B 之后仍然不流畅。实测 `_currentTime` 的粒度：

```
t=2516637  cur=807.757   ← 连续 11 次采样都不动
t=2517747  cur=808.757   ← 跳 1 秒
t=2518553  cur=808.757   ← 又不动了
```

即：源时间是**整秒跳变**的。原实现每收到快照就把时钟重置到该值，于是
「外推 200ms → 被拉回最多 1 秒 → 再外推」，视觉上就是来回抽搐。

修复（`src/main.ts` 的 `PlayClock`）：自己维护单调时钟，只把误差的 35%（且单次
最多 0.12s）缓释进去；误差 > 2s 才判定为 seek/换曲并直接跳转。

验证（每 80ms 采样 25 次）：

```
30.004 → 30.085 → 30.167 → … → 31.936 → 32.020     回退次数 0
```

## 验证结果

同刻比对：

```
浮窗时钟 pos = 209.038
Apple songPos = 209.000     → 误差 38 ms
当前行 = 又何必去改變已錯過的時間
```


日期：2026-11-10 23:1x–23:5x (UTC+8)
环境：CDP Chrome（独立 profile，已登录，订阅有效，storefront=cn）

## 结果

| 项 | 结果 |
|---|---|
| CDP 连上 Apple Music 页面 | ✅ `ws://127.0.0.1:9222/devtools/page/...` |
| 从页面读到实时播放位置 | ✅ `audio.currentTime`（毫秒级） |
| 拿到 Apple 自家歌词 | ✅ 47 行，`origin=apple level=Line` |
| 浮窗渲染 | ✅ 青色填充 + 黑描边，双行结构 |

## 1. 歌词端点是 `syllable-lyrics`，不是 `lyrics`

盲猜了一堆形态全部 404（`/lyrics`、`?l[lyrics]=default`、`amp-api-edge` 域名、带
`media-user-token` 也一样），错误码固定是：

```json
{"errors":[{"title":"No related resources","detail":"No related resources found for lyrics","status":404,"code":"40403"}]}
```

真正能用的路径是**从播放器自己的流量里抓到的**（`tools/cdp-netlog.mjs`）：

```
GET https://amp-api.music.apple.com/v1/catalog/{sf}/songs/{id}/syllable-lyrics
    ?l%5Blyrics%5D=zh-hans-cn&l%5Bscript%5D=zh-Hans&extend=ttmlLocalizations
Headers: Authorization: Bearer <developerToken>
         media-user-token: <musicUserToken>
         Origin: https://music.apple.com
```

响应：

```json
{ "data": [ { "id": "...", "type": "songs",
  "attributes": { "playParams": {...}, "ttmlLocalizations": "<tt ...>...</tt>" } } ] }
```

`ttmlLocalizations` **本身就是一个字符串**（不是对象），直接当 TTML 塞进解析器。

## 2. amp-web 的 MusicKit 没有 `.player`

`MusicKit.getInstance()` 存在、`developerToken` 可读、`musicUserToken` 可读，但
`inst.player` 是 `undefined`——它用的是 `services.player` / `_playbackController` 那套内部结构。
所以：

* 曲名/歌手改读 `navigator.mediaSession.metadata`（页面自己会设，SMTC 也是从这来的）；
* catalog id 不做内部结构挖掘，改用**搜索接口反查**：

```
GET /v1/catalog/{sf}/search?term=<title artist>&types=songs&limit=10
```

按「标题完全相同 > 标题包含」+「歌手包含」+「hasLyrics」打分选最佳，再取 `syllable-lyrics`。

## 3. 页面里有 7 个 `<audio>`，只有一个是真播

6 个是货架上的 30 秒试听（`dur=30.000998, paused=true`），第 7 个才是真流：

```
{"tag":"AUDIO","cur":66.155559,"dur":null,"paused":false}
```

`dur` 是 `null` —— MSE 流没有有限 duration。所以取媒体元素不能取第一个，要取「正在播且
currentTime > 0」的那个，按时长降序取最大。

## 4. 渲染踩坑：`text-shadow` 会吃掉 `background-clip: text` 的填充

现象：算出来的样式全对（`bgClip=text`、渐变在、`--p=1`），但屏幕上只有一坨黑。

隔离实验（同一页面注入三段文字）：

| 测试 | 结果 |
|---|---|
| 渐变 + `background-clip:text`，无描边 | ✅ 红→蓝渐变正常 |
| 纯色 | ✅ |
| 渐变 + `text-shadow` 描边 | ❌ 全黑，渐变完全不画 |

修复：**拆成两层**——底层是纯黑 `-webkit-text-stroke`，上层是渐变填充，两层用
`display:grid; grid-area:1/1` 叠在一起。同时把描边从 2px 降到 1.2px，否则中文细笔画会被吃掉。

## 5. 重载后歌词丢失

前端是事件驱动的，页面一重载（HMR 或重开窗口）事件就没了，歌词永远不回来。加了
`get_lyrics` 命令，前端启动时主动拉一次后端缓存的歌词集。

## 未解决

- `itunes:timing="Line"` 是常见情况；`"Word"` 的样本还没遇到，逐字路径未验证。
- 歌词是繁体原文 + `translations` 块（`zh-Hans` 替换文本），目前没应用翻译，显示的是原文。
- 播放器对象的 `duration` 拿不到（MSE），进度条长度暂时用不上。
