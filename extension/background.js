// NDM 下载助手：拦截浏览器下载并转交给 NDM（127.0.0.1:16800）

import { crawlOpenList, detectOpenList } from "./openlist.js";
import { DEFAULTS, shouldIntercept } from "./rules.js";
import { setupSniffer } from "./sniffer.js";

/** NDM 接入地址；端口可通过 chrome.storage.local 的 ndmPort 修改（开发测试用），默认 16800 */
async function ndmBase() {
  const { ndmPort } = await chrome.storage.local.get({ ndmPort: 16800 });
  return `http://127.0.0.1:${ndmPort}`;
}
/** 交还给浏览器的 URL（转交失败后重新下载时不再拦截） */
const bypass = new Set();

async function getConfig() {
  return { ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) };
}

async function ping(timeout = 800) {
  try {
    const r = await fetch(`${await ndmBase()}/ping`, { signal: AbortSignal.timeout(timeout) });
    return r.ok ? await r.json() : null;
  } catch {
    return null;
  }
}

async function cookiesFor(url) {
  try {
    const list = await chrome.cookies.getAll({ url });
    return list.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return "";
  }
}

async function sendToNdm({ url, referrer, filename, fileSize, hls = false }) {
  const alive = await ping();
  if (!alive?.token) throw new Error("NDM 未授权");
  const body = {
    url,
    referrer: referrer || "",
    filename: filename || "",
    fileSize: fileSize > 0 ? fileSize : null,
    cookies: await cookiesFor(url),
    userAgent: navigator.userAgent,
    hls,
  };
  const r = await fetch(`${await ndmBase()}/add`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-NDM-Token": alive.token },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(4000),
  });
  if (!r.ok) {
    const err = await r.json().catch(() => ({}));
    throw new Error(err.error || `HTTP ${r.status}`);
  }
}

async function setBadge(online) {
  await chrome.action.setBadgeText({ text: online ? "" : "!" });
  if (!online) await chrome.action.setBadgeBackgroundColor({ color: "#6b6b6b" });
}

/**
 * 在浏览器确定文件名（以及弹出“另存为”对话框）之前拦截。
 * 浏览器会等待 suggest() 被调用；不拦截时必须调用 suggest()，否则下载会卡住。
 */
chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  handleDownload(item, suggest);
  return true; // 异步调用 suggest
});

async function handleDownload(item, suggest) {
  let answered = false;
  const letBrowserHandle = () => {
    if (!answered) {
      answered = true;
      suggest();
    }
  };
  try {
    const url = item.finalUrl || item.url;
    if (bypass.delete(url)) return letBrowserHandle();

    const verdict = shouldIntercept(item, await getConfig(), chrome.runtime.id);
    if (!verdict.ok) return letBrowserHandle();

    // NDM 未运行时交给浏览器，不影响正常下载
    const alive = await ping();
    await setBadge(!!alive);
    if (!alive) return letBrowserHandle();

    await sendToNdm({ url, referrer: item.referrer, filename: item.filename, fileSize: item.totalBytes || item.fileSize });
    // 转交成功：取消浏览器下载并从下载列表移除，不再弹出另存为对话框
    answered = true;
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch (e) {
    console.warn("NDM: 转交失败，交还浏览器下载", e);
    letBrowserHandle();
  }
}

// ---------- 连接状态 ----------

/** 刷新工具栏角标：NDM 未运行时显示「!」，恢复后自动消失 */
async function refreshBadge() {
  await setBadge(!!(await ping()));
}

// 定时检测（MV3 后台会被挂起，用 alarms 唤醒；最短间隔 30 秒）
chrome.alarms.create("ndm-ping", { periodInMinutes: 0.5 });
chrome.alarms.onAlarm.addListener((a) => {
  if (a.name === "ndm-ping") refreshBadge();
});
chrome.runtime.onStartup.addListener(refreshBadge);
refreshBadge();

// ---------- OpenList 当前文件夹 ----------

/** 在标签页的页面主环境中执行函数（可读取页面变量与登录状态） */
async function runInPage(tabId, func, args = []) {
  const [res] = await chrome.scripting.executeScript({ target: { tabId }, world: "MAIN", func, args });
  return res?.result;
}

async function detectInTab(tabId) {
  try {
    return await runInPage(tabId, detectOpenList);
  } catch {
    return null; // 浏览器内部页面等无法注入
  }
}

/** 爬取当前文件夹并交给 NDM 的批量下载卡片 */
async function downloadOpenList(tab, recursive) {
  if (!(await ping())) return { error: "未检测到 NDM，请先启动 NDM" };
  let res;
  try {
    res = await runInPage(tab.id, crawlOpenList, [recursive]);
  } catch (e) {
    return { error: `无法读取页面：${e.message || e}` };
  }
  if (!res || res.error) return { error: res?.error || "读取文件列表失败" };
  if (!res.items.length) return { error: recursive ? "当前文件夹（含子文件夹）没有文件" : "当前文件夹没有文件" };
  try {
    const alive = await ping();
    if (!alive?.token) return { error: "NDM 未授权，请重试" };
    const r = await fetch(`${await ndmBase()}/batch`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-NDM-Token": alive.token },
      body: JSON.stringify({ source: "openlist", title: res.title, referrer: tab.url, items: res.items, truncated: res.truncated }),
      signal: AbortSignal.timeout(8000),
    });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) return { error: j.error || `HTTP ${r.status}` };
    return { ok: true, count: j.count ?? res.items.length, truncated: res.truncated };
  } catch (e) {
    return { error: `发送到 NDM 失败：${e.message || e}` };
  }
}

// 调试入口：可在扩展后台控制台调用 ndm.downloadOpenList(tab, recursive)
globalThis.ndm = { detectInTab, downloadOpenList, ping };

// ---------- 右键菜单 ----------

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: "ndm-link", title: "使用 NDM 下载链接", contexts: ["link"] });
  chrome.contextMenus.create({ id: "ndm-media", title: "使用 NDM 下载此文件", contexts: ["image", "video", "audio"] });
  chrome.contextMenus.create({ id: "ndm-openlist", title: "使用 NDM 下载当前文件夹（OpenList）", contexts: ["page"] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === "ndm-openlist") {
    if (!tab?.id) return;
    const res = (await detectInTab(tab.id)) ? await downloadOpenList(tab, false) : { error: "当前页面不是 OpenList 站点" };
    if (res.error) runInPage(tab.id, (m) => alert(m), [`NDM：${res.error}`]).catch(() => {});
    return;
  }
  const url = info.linkUrl || info.srcUrl;
  if (!url || !/^https?:\/\//i.test(url)) return;
  const alive = await ping();
  await setBadge(!!alive);
  if (!alive) {
    // NDM 未运行：退回浏览器下载
    bypass.add(url);
    chrome.downloads.download({ url });
    return;
  }
  try {
    await sendToNdm({ url, referrer: info.pageUrl });
  } catch (e) {
    console.warn("NDM: 转交失败", e);
  }
});

// 弹出面板查询连接状态
const handleMedia = setupSniffer({ getConfig, sendToNdm, runInPage });
chrome.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg?.type === "media-list" || msg?.type === "media-download") {
    handleMedia(msg).then(reply).catch(error => reply({ error: error.message || "视频资源操作失败" }));
    return true;
  }
  if (msg?.type === "openlist-detect") {
    detectInTab(msg.tabId).then(reply);
    return true;
  }
  if (msg?.type === "openlist-download") {
    chrome.tabs.get(msg.tabId).then((tab) => downloadOpenList(tab, !!msg.recursive)).then(reply);
    return true;
  }
  if (msg?.type === "ping") {
    ping().then(async (r) => {
      await setBadge(!!r);
      reply(r);
    });
    return true;
  }
});
