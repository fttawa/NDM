import { DEFAULTS } from "./rules.js";

const $ = (id) => document.getElementById(id);
const fields = Object.keys(DEFAULTS);

async function load() {
  const cfg = { ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) };
  for (const k of fields) {
    const el = $(k);
    if (el.type === "checkbox") el.checked = cfg[k];
    else el.value = cfg[k];
  }
}

function save(k) {
  const el = $(k);
  const v = el.type === "checkbox" ? el.checked : el.type === "number" ? Math.max(0, Number(el.value) || 0) : el.value;
  chrome.storage.sync.set({ [k]: v });
}

for (const k of fields) {
  $(k).addEventListener("change", () => save(k));
}

async function refreshStatus() {
  const r = await chrome.runtime.sendMessage({ type: "ping" });
  const el = $("status");
  el.className = `status ${r ? "ok" : "off"}`;
  el.textContent = r ? `已连接 NDM ${r.version}` : "未检测到 NDM，下载将由浏览器处理";
}

// OpenList：当前标签页是 OpenList 站点时显示批量下载入口
async function setupOpenList() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.id) return;
  const found = await chrome.runtime.sendMessage({ type: "openlist-detect", tabId: tab.id });
  if (!found) return;
  $("olKind").textContent = found.kind;
  $("openlist").hidden = false;
  const run = async (recursive) => {
    const msg = $("olMsg");
    $("olHere").disabled = $("olDeep").disabled = true;
    msg.className = "ol-msg";
    msg.textContent = recursive ? "正在读取文件列表（含子文件夹）…" : "正在读取文件列表…";
    const res = await chrome.runtime.sendMessage({ type: "openlist-download", tabId: tab.id, recursive });
    if (res?.ok) {
      msg.className = "ol-msg ok";
      msg.textContent = `已发送 ${res.count} 个文件到 NDM${res.truncated ? "（超过上限已截断）" : ""}`;
      setTimeout(() => window.close(), 900);
    } else {
      msg.className = "ol-msg err";
      msg.textContent = res?.error || "失败";
      $("olHere").disabled = $("olDeep").disabled = false;
    }
  };
  $("olHere").addEventListener("click", () => run(false));
  $("olDeep").addEventListener("click", () => run(true));
}

load();
refreshStatus();
setupOpenList();

async function refreshMedia() {
  const list = $("mediaList");
  const message = $("mediaMsg");
  $("mediaRefresh").disabled = true;
  message.textContent = "正在读取…";
  list.replaceChildren();
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    const items = tab?.id ? await chrome.runtime.sendMessage({ type: "media-list", tabId: tab.id }) : [];
    if (!Array.isArray(items)) throw new Error(items?.error || "读取失败");
    message.textContent = items.length ? `${items.length} 个资源 · HLS 默认选择最高可用清晰度` : "播放视频后刷新列表；暂不支持 blob、直播和加密 HLS";
    for (const item of items) {
      const row = document.createElement("div"); row.className = "media-item";
      const label = document.createElement("div"); label.className = "media-label";
      const name = document.createElement("span"); name.textContent = item.name; name.title = item.url;
      const meta = document.createElement("small"); meta.textContent = `${item.hls ? "HLS" : "视频"} · ${item.host}`;
      label.append(name, meta);
      const button = document.createElement("button"); button.textContent = "下载";
      button.addEventListener("click", async () => {
        button.disabled = true;
        try {
          const result = await chrome.runtime.sendMessage({ type: "media-download", tabId: tab.id, url: item.url });
          if (!result?.ok) throw new Error(result?.error || "转交失败");
          button.textContent = "已发送";
        } catch (error) { message.textContent = error.message; button.disabled = false; }
      });
      row.append(label, button); list.append(row);
    }
  } catch (error) { message.textContent = error.message || "读取失败"; }
  finally { $("mediaRefresh").disabled = false; }
}
$("mediaRefresh").addEventListener("click", refreshMedia);
$("videoSniff").addEventListener("change", () => setTimeout(refreshMedia, 100));
refreshMedia();
