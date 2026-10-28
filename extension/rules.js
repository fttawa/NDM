// 拦截规则（纯函数，便于测试）

export const DEFAULTS = {
  /** 是否拦截浏览器下载 */
  enabled: true,
  videoSniff: true,
  /** 小于该大小（KB）的文件交给浏览器，0 表示不限 */
  minSizeKB: 0,
  /** 只拦截这些扩展名（逗号/空格分隔），留空表示全部 */
  types: "",
  /** 不拦截的网站（逗号/空格/换行分隔），匹配域名及其子域名 */
  excludeSites: "",
};

export function splitList(text) {
  return (text || "")
    .split(/[\s,，;；]+/)
    .map((s) => s.trim().toLowerCase().replace(/^\./, ""))
    .filter(Boolean);
}

export function extOf(name) {
  const base = (name || "").split(/[\\/]/).pop().split(/[?#]/)[0];
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1).toLowerCase() : "";
}

export function hostMatches(host, site) {
  host = host.toLowerCase();
  site = site.replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/^\*\./, "");
  return host === site || host.endsWith("." + site);
}

/**
 * 判断一个浏览器下载是否应交给 NDM。
 * item 为 chrome.downloads.DownloadItem 的子集：url、finalUrl、filename、fileSize、totalBytes、incognito、byExtensionId
 * 返回 { ok: boolean, reason: string }
 */
export function shouldIntercept(item, cfg, selfId) {
  if (!cfg.enabled) return { ok: false, reason: "拦截已关闭" };
  const url = item.finalUrl || item.url || "";
  if (!/^https?:\/\//i.test(url)) return { ok: false, reason: "非 HTTP 链接（blob/data 等）" };
  if (item.incognito) return { ok: false, reason: "无痕窗口" };
  if (selfId && item.byExtensionId === selfId) return { ok: false, reason: "扩展自身发起" };

  const size = item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : 0;
  if (cfg.minSizeKB > 0 && size > 0 && size < cfg.minSizeKB * 1024) return { ok: false, reason: "小于最小大小" };

  let host = "";
  try {
    host = new URL(url).hostname;
  } catch {
    return { ok: false, reason: "链接无效" };
  }
  if (splitList(cfg.excludeSites).some((s) => hostMatches(host, s))) return { ok: false, reason: "网站在排除列表" };

  const types = splitList(cfg.types);
  if (types.length) {
    const ext = extOf(item.filename) || extOf(new URL(url).pathname);
    if (!types.includes(ext)) return { ok: false, reason: "文件类型不在列表" };
  }
  return { ok: true, reason: "" };
}
