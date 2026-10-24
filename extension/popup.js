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
