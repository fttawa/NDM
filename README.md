<p align="center">
  <img src="docs/banner.png" alt="NDM — 你的下一代开源高速下载器" width="100%" />
</p>

<p align="center">
  <a href="https://github.com/fttawa/NDM/releases"><img src="https://img.shields.io/github/v/tag/fttawa/NDM?label=version&color=3a3a3d&style=flat-square" alt="version" /></a>
  <img src="https://img.shields.io/badge/platform-Windows-3a3a3d?style=flat-square" alt="platform" />
  <img src="https://img.shields.io/badge/Tauri-2-3a3a3d?style=flat-square" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/Rust-engine-3a3a3d?style=flat-square" alt="Rust" />
</p>

<p align="center">
  多线程分段 · 动态分段 · 断点续传 · 浏览器接管 · OpenList 整目录下载
</p>

---

官网：https://fttawa.github.io/NDM/

NDM 是一个基于 Tauri 2 + Rust 的多线程下载管理器，目标是成为 IDM 的开源替代品。界面参考 Motrix，采用 Bento Grid 布局、深色中性灰与毛玻璃材质；安装包只有约 2 MB。

<p align="center">
  <img src="docs/screenshot.png" alt="NDM 主界面" width="100%" />
</p>

## ✨ 特性

| | |
| --- | --- |
| **多连接加速** | 每个任务 1–32 条连接，强制 HTTP/1.1，每个分段独占一条 TCP 连接 |
| **动态分段** | 空闲连接自动拆分剩余最多的分段接手，跑到最后也不掉速 |
| **连接超频** | 实测请求到服务器开始传输的延迟，预判即将传完或重试中的连接，提前补上新连接（最多加开连接数的一半）防止掉速 |
| **断点续传** | 分段进度持久化，崩溃或重启后从断点继续；服务器不支持 Range 时自动降级单连接 |
| **浏览器接管** | Chrome / Edge 扩展在「另存为」弹出前拦截下载，连同 Cookie、Referer、UA 一起转交 |
| **IDM 式小卡片** | 拦截后弹出置顶确认卡片，点击「开始下载」原地展开为进度窗口，不打扰主窗口 |
| **OpenList / AList** | 一键爬取当前文件夹（可含子文件夹），勾选后批量下载，保留目录结构与真实文件名 |
| **批量管理** | 复选框多选，Shift 连选、Ctrl+A 全选，批量开始 / 暂停 / 重新下载 / 删除 |
| **网络** | 全局限速、并发队列、系统代理 / 手动 HTTP · SOCKS5 代理 |
| **桌面体验** | 系统托盘、完成通知、剪贴板监听、亚克力 / 云母窗口材质 |

## 📦 安装

前往 [Releases](https://github.com/fttawa/NDM/releases) 下载 `NDM_x.y.z_x64-setup.exe` 安装即可（Windows 10 / 11，需要 WebView2，系统一般已自带）。

## 🧩 浏览器扩展

`extension/` 是 Chrome / Edge 扩展（MV3），安装包里也附带一份（设置 → 浏览器集成 → 打开扩展目录）。

1. 打开 `chrome://extensions`（Edge 为 `edge://extensions`），开启「开发者模式」
2. 点击「加载已解压的扩展程序」，选择扩展目录

扩展功能：

- 浏览器下载开始前拦截并转交 NDM；NDM 未运行时自动交还浏览器下载
- 右键链接 / 图片 / 视频：「使用 NDM 下载」
- 弹出面板可设置拦截开关、最小文件大小、文件类型、排除网站
- 在 OpenList / AList 页面，从扩展弹窗或右键菜单「下载当前文件夹」，使用当前登录身份与文件夹密码读取列表

> NDM 通过 `127.0.0.1:16800` 接收扩展请求，只接受来自浏览器扩展或本机程序的请求，普通网页无法添加任务。

## 🛠️ 开发

```bash
pnpm install
pnpm tauri dev
```

只调 UI 时可以运行 `pnpm dev`，在浏览器中打开 http://localhost:1420（使用内置 mock 数据）。

| 环境变量 | 作用 |
| --- | --- |
| `NDM_DATA_DIR` | 使用独立数据目录，同时跳过单实例限制，便于与已安装的 NDM 并行测试 |
| `NDM_BRIDGE_PORT` | 更换扩展接入端口（默认 16800） |

测试：`cargo test`（在 `src-tauri/` 下，下载引擎分段逻辑）、`node extension/rules.test.mjs`（扩展拦截规则）。

### 打包

```bash
pnpm tauri build
```

产物位于 `src-tauri/target/release/bundle/nsis/`。

### 项目结构

```
src/            React 19 前端（zustand、lucide-react）
src-tauri/      Rust 后端：下载引擎、任务管理、扩展桥接、确认卡片窗口
extension/      Chrome / Edge 扩展（MV3）
docs/           官网（GitHub Pages）与 README 图片，头图源文件在 docs/banner/
```

## 📁 数据位置

任务与设置保存在 `%APPDATA%\com.ndm.app\`（`tasks.json`、`settings.json`）。
