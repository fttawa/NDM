// Pure detection rules shared by the network observer, page scanner, and tests.
export function mediaCandidate(url, contentType = "") {
  try {
    const parsed = new URL(url);
    if (!/^https?:$/.test(parsed.protocol) || parsed.username || parsed.password || url.length > 8192) return null;
    const mime = contentType.split(";")[0].trim().toLowerCase();
    const hls = /\.m3u8$/i.test(parsed.pathname) || ["application/vnd.apple.mpegurl", "application/x-mpegurl", "audio/mpegurl", "audio/x-mpegurl"].includes(mime);
    const file = /\.(mp4|webm|mov|m4v)$/i.test(parsed.pathname) || ["video/mp4", "video/webm", "video/quicktime"].includes(mime);
    if (!hls && !file) return null;
    // Individual MSE/fMP4 chunks are not standalone videos.
    if (!hls && /\.(m4s|ts|aac)$/i.test(parsed.pathname)) return null;
    let name = parsed.pathname.split("/").pop() || "video";
    try { name = decodeURIComponent(name); } catch { /* retain encoded filename */ }
    return { url: parsed.href, hls, name: name.slice(0, 160), host: parsed.host };
  } catch { return null; }
}

export function mergeMedia(items, candidate) {
  const existing = items.find(item => item.url === candidate.url);
  if (existing) return items.map(item => item === existing ? { ...item, hls: item.hls || candidate.hls } : item);
  return [...items, candidate].slice(-100);
}
