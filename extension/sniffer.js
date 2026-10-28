import { mediaCandidate, mergeMedia } from "./media.js";
import { splitList } from "./rules.js";

export function setupSniffer({ getConfig, sendToNdm, runInPage }) {
  const queues = new Map();
  const epochs = new Map();
  const requests = new Map();
  const key = tabId => `media:${tabId}`;
  const enqueue = (tabId, action) => {
    const pending = (queues.get(tabId) || Promise.resolve()).then(action).catch(() => {});
    queues.set(tabId, pending);
    pending.finally(() => { if (queues.get(tabId) === pending) queues.delete(tabId); });
    return pending;
  };
  const allowed = async tab => {
    if (!tab || tab.incognito || !/^https?:\/\//i.test(tab.url || "")) return false;
    const config = await getConfig();
    const host = new URL(tab.url).hostname;
    return config.videoSniff && !splitList(config.excludeSites).some(site => host === site || host.endsWith(`.${site}`));
  };
  const remember = async (tabId, candidate, epoch) => {
    const tab = await chrome.tabs.get(tabId);
    if (!await allowed(tab) || epochs.get(tabId) !== epoch) return;
    await enqueue(tabId, async () => {
      if (epochs.get(tabId) !== epoch) return;
      const storageKey = key(tabId);
      const stored = await chrome.storage.session.get(storageKey);
      await chrome.storage.session.set({ [storageKey]: mergeMedia(stored[storageKey] || [], candidate) });
    });
  };
  chrome.webRequest.onBeforeRequest.addListener(details => {
    if (details.tabId < 0 || details.incognito) return;
    if (details.type === "main_frame") {
      epochs.set(details.tabId, (epochs.get(details.tabId) || 0) + 1);
      enqueue(details.tabId, () => chrome.storage.session.remove(key(details.tabId)));
    }
    // A worker wake-up without a main-frame event still has a stable generation.
    if (!epochs.has(details.tabId)) epochs.set(details.tabId, 0);
    requests.set(details.requestId, epochs.get(details.tabId));
  }, { urls: ["http://*/*", "https://*/*"] });
  chrome.webRequest.onHeadersReceived.addListener(details => {
    if (details.tabId < 0 || details.incognito || details.statusCode < 200 || details.statusCode >= 300) return;
    const mime = details.responseHeaders?.find(header => header.name.toLowerCase() === "content-type")?.value || "";
    const candidate = mediaCandidate(details.url, mime);
    if (candidate) remember(details.tabId, candidate, requests.get(details.requestId)).catch(() => {});
  }, { urls: ["http://*/*", "https://*/*"] }, ["responseHeaders"]);
  chrome.webRequest.onCompleted.addListener(details => {
    requests.delete(details.requestId);
  }, { urls: ["http://*/*", "https://*/*"] });
  chrome.webRequest.onErrorOccurred.addListener(details => requests.delete(details.requestId), { urls: ["http://*/*", "https://*/*"] });
  chrome.tabs.onRemoved.addListener(tabId => {
    epochs.delete(tabId);
    enqueue(tabId, () => chrome.storage.session.remove(key(tabId)));
  });
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "sync" && (changes.videoSniff || changes.excludeSites)) {
      for (const tabId of epochs.keys()) {
        epochs.set(tabId, epochs.get(tabId) + 1);
        enqueue(tabId, () => chrome.storage.session.remove(key(tabId)));
      }
    }
  });
  return async message => {
    const tab = await chrome.tabs.get(message.tabId);
    if (!await allowed(tab)) return message.type === "media-list" ? [] : { error: "该页面未启用视频嗅探" };
    if (!epochs.has(tab.id)) epochs.set(tab.id, 0);
    if (message.type === "media-list") {
      const epoch = epochs.get(tab.id);
      // Covers <video src> resources that were loaded before the extension started.
      const sources = await runInPage(tab.id, () => [...document.querySelectorAll("video, video source")].map(element => element.currentSrc || element.src).filter(Boolean).slice(0, 100)).catch(() => []);
      for (const url of sources || []) {
        const candidate = mediaCandidate(url);
        if (candidate) await remember(tab.id, candidate, epoch);
      }
      await (queues.get(tab.id) || Promise.resolve());
      if (epochs.get(tab.id) !== epoch) return [];
      return (await chrome.storage.session.get(key(tab.id)))[key(tab.id)] || [];
    }
    const candidate = ((await chrome.storage.session.get(key(tab.id)))[key(tab.id)] || []).find(item => item.url === message.url);
    if (!candidate) return { error: "资源已失效，请刷新嗅探列表" };
    await sendToNdm({ url: candidate.url, referrer: tab.url, hls: candidate.hls });
    return { ok: true };
  };
}
