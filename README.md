# NDM

基于 Tauri 2 的多线程下载管理器（IDM 替代品），界面参考 Motrix，采用 Bento Grid 布局 + 深色毛玻璃风格。

## 特性

- 多连接分段下载（每任务 1–32 连接，强制 HTTP/1.1，每段独占一条 TCP 连接）
- 动态分段：空闲连接自动拆分剩余最多的分段接手
- 断点续传：分段进度持久化，崩溃/重启后从断点继续
- 服务器不支持 Range 时自动降级为单连接
- 全局限速、任务队列（并发上限）、系统托盘、完成通知、剪贴板监听
- 窗口材质：亚克力 / 云母 / 关闭（Windows）

## 浏览器扩展

`extension/` 是 Chrome / Edge 扩展（MV3），安装包会附带一份（设置 → 浏览器集成 → 扩展目录）。

- 在浏览器下载开始前（另存为对话框弹出前）拦截，连同 Cookie、Referer、User-Agent 转交 NDM
- 拦截后弹出置顶的下载确认小卡片（类似 IDM），不打扰主窗口
- 右键链接 / 图片 / 视频：「使用 NDM 下载」
- 弹出面板可设置：拦截开关、最小文件大小、文件类型、排除网站
- NDM 未运行时自动交还浏览器下载
- OpenList / AList：在扩展弹窗（或网页右键菜单）中「下载当前文件夹」，可包含子文件夹；使用当前登录身份与文件夹密码读取列表，在 NDM 中勾选后批量下载，保留目录结构、使用列表中的真实文件名

安装：打开 `chrome://extensions`（Edge 为 `edge://extensions`），开启「开发者模式」，「加载已解压的扩展程序」选择扩展目录。

NDM 通过 `127.0.0.1:16800` 接收扩展请求，只接受来自浏览器扩展或本机程序的请求，普通网页无法添加任务。

## 开发

```bash
pnpm install
pnpm tauri dev
```

只调 UI 时可以运行 `pnpm dev`，在浏览器中打开 http://localhost:1420（使用内置 mock 数据）。

测试实例：设置 `NDM_DATA_DIR=<目录>` 使用独立数据目录（不受单实例限制），`NDM_BRIDGE_PORT=<端口>` 换用其他扩展接入端口，避免与正在运行的 NDM 冲突。扩展规则测试：`node extension/rules.test.mjs`。

## 打包

```bash
pnpm tauri build
```

产物位于 `src-tauri/target/release/bundle/nsis/`。

## 数据位置

任务与设置保存在 `%APPDATA%\com.ndm.app\`（`tasks.json`、`settings.json`）。
