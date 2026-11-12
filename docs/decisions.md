# 设计决策记录

本文件记录 LyricFloat 在设计拷问阶段定下的每个分支，以及当时被否决的选项。改动要在这里留痕。

状态：`已定` / `待验证` / `已推翻`

| # | 分支 | 决定 | 备选与被否原因 | 状态 |
|---|---|---|---|---|
| 1 | 架构分层 | 浮窗（原生）与供料（provider）解耦，供料可插拔 | 单一供料路径不行：SMTC 精度存疑、CDP 依赖浏览器启动方式，任一失手整个项目停摆 | 已定 |
| 2 | 工程栈 | Tauri 2 + WebView2 | Electron 体积/内存翻倍；C#/WPF 做不了逐字渐变（`background-clip: text`） | 已定 |
| 3 | 歌词源 | 三层瀑布：Apple TTML → lrclib → 手动粘贴 | 只用 lrclib 缺逐字；只用 TTML 没版权就空白；加网易云/QQ 非官方接口维护成本过高 | 已定 |
| 4 | 浮窗交互 | 双态：默认鼠标穿透，热键临时接管，停手 5 秒回落 | 始终可交互会挡住下面的浏览器；始终穿透改位置要开设置窗 | 已定 |
| 5 | 显示规格 | 双行：当前行逐字渐变 + 下一行预告；无 TTML 时退化为整行点亮 | 单行看不到下文；多行滚动实现最重 | 已定 |
| 6 | 热键 | 两键一套：`` Alt+` `` 显隐、`Alt+M` 切穿透、`Alt+[` `Alt+]` 字号、`Alt+,` `Alt+.` 偏移、`Alt+P` 播放暂停；面板可改 | `Ctrl+\`` 被 VSCode/输入法抢；`Alt+Space` 是系统窗口菜单；自由录制 UI 多一天工作量 | 已定 |
| 7 | CDP 接入 | 内置按钮用专用 profile + 调试端口拉起 Chrome，直接打开 music.apple.com，登录态一次保留 | 复用日常 profile 需完全退出 Chrome，与日常使用互相干扰 | 已定 |
| 8 | 会话消歧 | 优先 Apple Music 会话，多个取"在播且最近更新"，托盘可手动钉住 | 见下方修订 | **已推翻** |
| 9 | 交付形态 | 开源仓库 + 安装包 + 便携版，CI 出包 | 只做 dev 工程以后要返工目录结构 | 已定 |
| 10 | 命名 | 仓库 `lyric-float`，显示名「浮词」 | 名字里带 Apple Music 有商标风险 | 已定 |
| 11 | 预设默认 | 配置存 `%APPDATA%\lyric-float\config.json` 可手改、开机自启关、托盘常驻、单实例、MIT、右下角、透明度 0.85、思源黑体→微软雅黑、2px 描边 | — | 已定 |

## 修订记录

### #8 消歧：已推翻，M0 实测导致

原规则"优先取来源为 `music.apple.com` 的会话"在 SMTC 模式下**无法实现**：
`SourceAppUserModelId` 只有 `Chrome.<hash>`，不含 URL；且主 Chrome 与隔离 profile 的 Chrome
拿到的是同一个 hash（见 `docs/m0-smtc-probe.md`）。

新规则：

1. 只接受 AUMID 前缀为 `Chrome.` / `MsEdge.` 的会话；
2. 多个候选时，取 `PlaybackStatus == Playing` 且 `LastUpdatedTime` 最新的一个；
3. 托盘菜单列出全部会话（应用名 + 曲名），允许手动钉住，钉住优先于自动判断；
4. CDP 模式可用时，用真实 URL 覆盖以上判断（域名级精确）。

## 未决项

- **Apple Music 网页版是否真的调 `setPositionState`** —— M0 唯一剩下的未知，1 分钟可验（见 m0 文档）。
  不通过则 SMTC 降级为"只提供曲名/状态"，进度一律走 CDP。
- 逐字歌词的 TTML 解析粒度（syllable vs word），需要抓到真实 TTML 样本才能定。
