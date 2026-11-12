# LyricFloat / 浮词

给**网页版 Apple Music**（以及任何会向 Windows 报告媒体会话的播放器）加一层桌面歌词浮窗。
透明、置顶、鼠标穿透、全局热键，逐字渐变高亮。

> 状态：**M3 打通**——CDP 读页面内 `audio.currentTime` 拿毫秒级进度，用 Apple 自己的
> `syllable-lyrics` 接口取 TTML 歌词，浮窗上已能逐行滚动、青色填充 + 黑描边。
> 单测 19/19。见 [docs/m0-smtc-probe.md](docs/m0-smtc-probe.md)、
> [docs/m1-build-notes.md](docs/m1-build-notes.md)、[docs/m2-session-notes.md](docs/m2-session-notes.md)、
> [docs/m3-apple-lyrics.md](docs/m3-apple-lyrics.md) 与 [docs/decisions.md](docs/decisions.md)。

## 它怎么工作

```
┌──────────── 供料层（可插拔）────────────┐        ┌──── 渲染层 ────┐
│ SMTC   Windows 媒体会话，零配置         │        │ Tauri + WebView2 │
│ CDP    Chrome 调试端口，毫秒级 + TTML   │──────▶ │ 透明置顶窗        │
│ 手动   用户贴一份 LRC                   │ 事件   │ 双行逐字渐变      │
└──────────────────────────────────────┘        └──────────────────┘
```

歌词三级瀑布：**Apple TTML（逐字）→ lrclib（逐行）→ 手动粘贴**。

## 实测定论（M0）

| 能力 | 结果 |
|---|---|
| 曲名 / 歌手 / 专辑 | ✅ |
| 播放状态 / 曲长 | ✅ |
| 当前进度 | ✅ 上报年龄 17–243 ms，速率比 0.991 |
| 来源域名 | ❌ AUMID 只有 `Chrome.<hash>`，没有 URL |

结论：SMTC 够逐行歌词用，逐字需要 CDP。详见 [docs/m0-smtc-probe.md](docs/m0-smtc-probe.md)。

## 自己验一遍（1 分钟，不需要装任何东西）

打开网页版 Apple Music 播一首歌，然后：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\smtc-probe\probe.ps1 -Samples 4 -IntervalMs 1000
```

看到 `STATUS Playing`、`pos` 逐次递增、`lastUpdated 距今 < 1s` 就说明你这边能跑。

受控对照实验（会短暂出声，自动清理）：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\smtc-probe\run-local-player.ps1
```

## 安装 / 运行

两种形态，产物分开：

| 形态 | 需要什么 | 产物 |
|---|---|---|
| **A. 独立浏览器版（0.1.x）** | 用自带启动器起的 Chrome（独立 profile，登录一次长期有效） | `LyricFloat_0.1.0_x64-setup.exe` + `launch.ps1` |
| **B. 扩展版（0.2.x，推荐）** | 装一个 Chrome 扩展，用**你自己的 Chrome** 听歌 | `lyric-float-0.2.0-ext-win64.exe` + `lyric-float-extension-0.2.0.zip` |

### B. 扩展版怎么用

1. Chrome 打开 `chrome://extensions` → 右上角开「开发者模式」→「加载已解压的扩展程序」→ 选中本仓库的 `extension/` 目录（或解压 zip 后的目录）。
2. 用你日常的 Chrome 打开 `music.apple.com` 播歌。
3. 启动 LyricFloat（`config.json` 里 `ext_enabled` 默认 `true`，端口 `8787`）。

浮窗改成接收扩展推来的播放状态和歌词，**不再需要调试端口、也不再需要第二个浏览器**。
扩展和浮窗之间的约定：`ws://127.0.0.1:8787/<token>`，token 默认 `lyricfloat`，
在 `extension/bridge.js` 与 `config.json` 的 `ext_token` 两处保持一致即可。

A 的最小用法是双击 `deploy\launch.ps1` 的桌面快捷方式：它先确认调试端口在听，没有就用独立
profile 起 Chrome，然后启动浮窗。

- 安装包：`src-tauri/target/release/bundle/nsis/LyricFloat_0.2.0_x64-setup.exe`（约 1.4 MB）
- 免安装：`src-tauri/target/release/lyric-float.exe`（约 3.9 MB）
- release 版没有控制台窗口；debug 版会带一个黑框（`windows_subsystem` 的区别）。

首次运行前，确保 `%APPDATA%\lyric-float\config.json` 里的 `webview_data_dir` 指向一个可写目录
（见下方已知问题 2），并把 `cdp_enabled` 设为 `true`、`cdp_port` 与启动器一致。

## 开发

前置：Rust（`rustup` + MSVC 生成工具）、Node 20+、pnpm、WebView2 运行时（Win10/11 一般自带）。

```bash
pnpm install
pnpm app:dev      # 开发运行
pnpm app:build    # 出安装包（NSIS + MSI）
cargo test --manifest-path src-tauri/Cargo.toml   # LRC/TTML 解析器单测
```

图标（仓库里没有二进制图标，首次需要生成）：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools\make-icons.ps1
pnpm tauri icon assets/logo.png
```

## 热键（默认，`config.json` 里可改）

| 键 | 作用 |
|---|---|
| `Alt+\`` | 显示 / 隐藏 |
| `Alt+M` | 进入 / 退出「调整位置」模式（5 秒后自动退出） |
| `Alt+←→↑↓` | **调整位置**（仅在 `Alt+M` 模式里生效，每次 12px 并立即存盘） |
| `Alt+[` / `Alt+]` | 字号 |
| `Alt+,` / `Alt+.` | 歌词偏移 ±0.1s |

配置：`%APPDATA%\lyric-float\config.json`，手改即可，不用重编译。

## 已知问题

1. **Apple Music 网页版是否调用 `setPositionState` 尚未验证。** 这是 M0 唯一剩下的未知；
   若不调用，SMTC 模式的进度就是死的，必须走 CDP。
2. **WebView2 的 profile 目录必须是可写路径。** Tauri 在 Windows 上强制把它设成
   `%LOCALAPPDATA%\<identifier>`；该路径不可用时 WebView2 创建即崩（`0x8000FFFF`，事件日志里是
   `msedge.dll` 的 `STATUS_BREAKPOINT`）。用 `config.json` 的 `webview_data_dir` 指到可写目录即可。
3. **`Alt+\`` 常被别的程序占用**（Utools / 输入法 / 录屏工具都会抢）。程序会自动回退到 `Alt+L`，
   并在控制台打出实际绑定的键。
4. 窗口可能出现在 Alt+Tab 列表里（`WS_EX_TOOLWINDOW` 未置位）。待修。
5. **点击穿透必须作用到子窗口**：WebView2 把内容放在独立子窗口里，只给顶层窗口设
   `WS_EX_TRANSPARENT` 是无效的。现在会把样式刷到整棵窗口树，并每 700ms 重刷一次
   （WebView2 导航会重建子窗口）。
6. **这个透明窗口一旦去掉 `WS_EX_TRANSPARENT` 就停止合成**：页面照常渲染（CDP 截图能看到文字），
   但屏幕上完全透明。所以「取消穿透 + 拖动」这条路走不通，位置改用 `Alt+M` + 方向键调整；
   窗口在任何状态下都保持穿透。
5. WebView2 透明窗在部分核显上会黑底或闪烁，需要 GPU 合成回退开关。

## 许可

MIT。项目名故意不含 "Apple Music"；Apple Music 是 Apple Inc. 的商标，本项目与其无关联。
