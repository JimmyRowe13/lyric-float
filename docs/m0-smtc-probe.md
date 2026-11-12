# M0 — SMTC 供料能力实测

日期：2026-11-10 00:44–00:46 (UTC+8)
机器：Windows 10 (22000)、Chrome 128.0.2739.54、Windows PowerShell 5.1
脚本：`tools/smtc-probe/probe.ps1`、`tools/smtc-probe/run-local-player.ps1`

## 结论

**PASS。** Windows SMTC（`GlobalSystemMediaTransportControlsSessionManager`）能给出一条正在播放的
Chromium 会话的：

| 字段 | 是否可用 | 说明 |
|---|---|---|
| 曲名 / 歌手 / 专辑 | ✅ | `TryGetMediaPropertiesAsync()` |
| 播放状态 | ✅ | `GetPlaybackInfo().PlaybackStatus` |
| 曲长 | ✅ | `GetTimelineProperties().EndTime` |
| 当前进度 | ✅ | `GetTimelineProperties().Position` |
| 进度新鲜度 | ✅ | `GetTimelineProperties().LastUpdatedTime` |
| 来源 URL / 域名 | ❌ | 只有 `SourceAppUserModelId`，形如 `Chrome.AAZS3A6TMWYPI7NBWIMNFUAHSQ` |

精度：进度上报年龄 17–243 ms，速率比 0.991（Δpos/Δwall），量化与声卡时钟抖动合计约 ±250 ms。
逐行歌词（容差 ~±150 ms）够用；逐字歌词（容差 ~±50 ms）需 CDP 补精度。

## 三次实验

### 1）被动读取用户现有会话（王菲《约定》）

```
APP      Chrome.AAZS3A6TMWYPI7NBWIMNFUAHSQ
STATUS   Paused
pos=0.000 start=0.000 end=267.635
lastUpdated 2026-11-08T23:19:53Z+08  (距今 91,460 s)
```

曲长正确，但会话是 25 小时前的残留：进度不刷新、`PlaybackStatus=Paused`。

### 2）`TryPlayAsync()` 驱动僵尸会话 —— 失败

```
DRIVE_PLAY ok=True
SAMPLE i=0..2 status=Paused   pos=0.000
SAMPLE i=3..7 status=Closed   end=0.000  lastUpdated=1601-01-01
```

`TryPlayAsync()` 返回 `True` 但会话直接消亡。**教训：命令返回值不可信，必须回读状态确认。**

### 3）受控本地播放页（隔离 profile，决定性实验）

`tools/smtc-probe/sandbox/player.html`：`<audio>` + `navigator.mediaSession.metadata`
+ 每 250 ms 调一次 `setPositionState({duration, playbackRate, position})` —— 与真实网页播放器同构。

```
STATUS Playing   RATE 1.0   end=15.000
i=0  pos=4.732   lastUpdated 距今 0.046s
i=1  pos=5.477   lastUpdated 距今 0.017s
i=2  pos=5.992   lastUpdated 距今 0.226s
...
i=11 pos=12.489  lastUpdated 距今 0.124s
```

进度随播放推进，`LastUpdatedTime` 持续刷新。**Chrome 确实把 MediaSession 的
`setPositionState` 转发进了 SMTC。**

## 由此确定的三条实现约束

1. **不能靠域名消歧。** AUMID 里没有 URL，而且主 Chrome 与隔离 profile 的 Chrome 拿到的是
   **同一个** `Chrome.AAZS3A6TMWYPI7NBWIMNFUAHSQ`。域名级识别只在 CDP 模式下成立。
   SMTC 模式下的消歧只能靠：曲名/歌手/专辑 + 用户手动钉住 + 可选目录校验。
2. **必须做新鲜度门控。** 页面停止上报后 SMTC 会保留旧值（实验 1 里足足放了 25 小时）。
   规则：`状态 != Playing` 时直接用原始 `Position`，不外推；`状态 == Playing` 且
   `age > 400ms` 才外推 `pos + age × rate`；`age > 3000ms` 判为失联，浮窗显示"未连接"而不是
   在错误的歌词上继续走。
3. **命令要回读。** 任何 `TryPlayAsync` / `TryPauseAsync` / `TrySkipNextAsync` 之后必须重新
   `GetPlaybackInfo()` 确认状态真的变了。

## 尚未验证的最后一个未知

**Apple Music 网页版自己到底调不调 `setPositionState`。** 实验 3 的页面是我写的，我让它调它就调；
Apple Music 是黑盒。若它不调，SMTC 模式下进度就是死的，必须切 CDP。

验证方法（1 分钟）：打开 music.apple.com 播一首，然后

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\smtc-probe\probe.ps1 -Samples 4 -IntervalMs 1000
```

判定：`STATUS Playing` 且 `lastUpdated 距今 < 1s` 且 `pos` 逐次递增 → 通过。

## 复现

```powershell
# 被动观测当前系统里所有媒体会话
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\smtc-probe\probe.ps1 -Samples 4

# 受控实验：起隔离 profile 的 Chrome 播 15s 正弦音，采样，然后杀干净
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\smtc-probe\run-local-player.ps1
```

`run-local-player.ps1` 用 `Get-CimInstance Win32_Process` 按 `--user-data-dir` 精确匹配来清理，
不会碰到你日常在用的 Chrome。
