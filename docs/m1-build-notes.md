# M1 — 实机编译与运行记录

日期：2026-11-10 22:0x–22:2x (UTC+8)
机器：Windows 11（build 22000 系）、Chrome 128、Edge/WebView2 Runtime 128.0.2739.54
工具链：rustc/cargo 1.99.1（`D:\rust`）、pnpm 11.28.5、node 24.15.0、MSVC 2022 (D:\VisualStudio\2022\Community) + Windows SDK 10.0.19041

## 结果

| 项 | 状态 |
|---|---|
| `cargo check` | ✅ 通过（0 error） |
| `cargo test` | ✅ 13/13 通过（LRC 7、TTML 4、会话排序 2） |
| 前端 `tsc --noEmit` + `vite build` | ✅ 通过 |
| 应用启动 | ✅ 运行中，WebView2 子进程 6 个 |
| 窗口属性 | ✅ `WS_EX_TOPMOST` + `WS_EX_TRANSPARENT`（点击穿透）+ `WS_EX_LAYERED`（透明），RECT 500,760 900×200 |
| 全局热键 | ✅ 6 个全部绑定（含冲突回退） |
| 真实歌词渲染 | ⛔ 未验证（还没有接上真实播放会话） |

## 踩到的坑（都已修，写下来别再踩）

### 1. WebView2 的 user data 目录决定生死

现象：`setup` 里报 `Failed to setup app: 拒绝访问 (os error 5)`，随后改为
`WebView2 error: WindowsError(HRESULT(0x8000FFFF))`，事件日志里 `msedgewebview2.exe`
以 `msedge.dll` 崩在 `0x80000003`（STATUS_BREAKPOINT，即 Chromium 的 `CHECK` 失败）。

二分结论（用 `tools/../.tmp-build/wv2test` 里的 30 行 .NET 宿主复现，与 Tauri 无关）：

| user data 目录 | 结果 |
|---|---|
| 默认（exe 旁边） | ✅ |
| `E:\wsl\codexproj\.tmp-build\wv2data` | ✅ 稳定 |
| `%LOCALAPPDATA%\dev.lyricfloat.overlay` | ❌ 稳定失败 |
| `C:\wv2data` / `C:\Users\jimmy\wv2data` / `C:\...\Temp\wv2data` | ❌ |
| `D:\wv2data` / `D:\wv2data2` | ❌ |
| `E:\wv2data`（盘根） | ❌ |

即：只有位于 `E:\wsl\codexproj\.tmp-build` 这类**已被沙箱 ACL 显式授权的目录**才成功
（该目录的 ACL 里有 `R9KP\CodexSandboxUsers` 以及若干特殊 SID）。原因链与本机
`CodexSandboxService` 的受限令牌有关，属环境特异问题，不是代码缺陷。

应对：Tauri 在 Windows 上**强制**把 WebView2 数据目录设成 `app_local_data_dir()`，
没有配置项可覆盖。所以改成在 `setup` 里用 `WebviewWindowBuilder::data_directory()`
自建窗口，并把路径做成配置项 `webview_data_dir`。

本机实测可用的 `%APPDATA%\lyric-float\config.json`：

```json
{ "webview_data_dir": "E:\\wsl\\codexproj\\.tmp-build\\lf-webview2" }
```

> 注意：PowerShell 5.1 的 `Set-Content -Encoding UTF8` 会写 BOM，`serde_json` 直接
> 报 `expected value at line 1 column 1`。必须用 `UTF8Encoding($false)` 无 BOM 写。

### 2. 热键：一个键失败不能毁掉全部

原实现里 `register_hotkeys` 返回 `Result`，第一个 `?` 失败就整体返回，
导致后面 5 个热键一个都没注册。现在改成逐个绑定、失败只记日志、并支持候选回退。

本机 `Alt+\``（Backquote）已被别的程序占用（`HotKey already registered, id 65536`），
自动回退到 `Alt+L`。

### 3. `Code` 枚举不认 `"L"`

`tauri_plugin_global_shortcut::Code` 用的是 WebIDL 命名（`KeyL`、`Digit1`、`ArrowUp`）。
配置文件里写 `Alt+L` 会解析失败且**没有任何提示**（`code?` 静默返回 None）。
已加 `code_from_token()` 做归一化，并在解析失败时打日志。

## 未解决

- 窗口 `WS_EX_TOOLWINDOW` 未置位，`skipTaskbar: true` 在自建窗口路径上可能没生效，
  表现为可能出现在 Alt+Tab 里。待查。
- 应用未在 release（`windows_subsystem = "windows"`）下验证，debug 构建会带一个控制台窗口。
- 歌词渲染仍只有假数据路径，接上真实 SMTC 会话后才算 M2 完成。

## 复现

```powershell
$env:RUSTUP_HOME='D:\rust\rustup'; $env:CARGO_HOME='D:\rust\cargo'
$env:TEMP='E:\wsl\codexproj\.tmp-build'; $env:TMP='E:\wsl\codexproj\.tmp-build'
$env:CARGO_TARGET_DIR='E:\wsl\codexproj\lyric-float\src-tauri\target'
cargo test --manifest-path E:\wsl\codexproj\lyric-float\src-tauri\Cargo.toml
powershell -File E:\wsl\codexproj\lyric-float\tools\dev-run.ps1 -WaitSeconds 12
```
