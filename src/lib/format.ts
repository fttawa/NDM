const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function bytes(n: number | null | undefined, digits = 1): string {
  if (n == null || !isFinite(n)) return "--";
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v.toFixed(0) : v.toFixed(digits)} ${UNITS[i]}`;
}

export function speed(n: number): string {
  return `${bytes(n)}/s`;
}

/** 拆分数值与单位，便于大号数字排版 */
export function speedParts(n: number): [string, string] {
  const s = bytes(n);
  const [v, u] = s.split(" ");
  return [v, `${u}/s`];
}

export function eta(remaining: number, bps: number): string {
  if (!bps || bps <= 0 || !isFinite(remaining)) return "--";
  let s = Math.round(remaining / bps);
  if (s < 60) return `${s} 秒`;
  const h = Math.floor(s / 3600);
  s %= 3600;
  const m = Math.floor(s / 60);
  s %= 60;
  if (h > 0) return `${h} 小时 ${m} 分`;
  return `${m} 分 ${s} 秒`;
}

export function percent(done: number, total: number | null): number {
  if (!total) return 0;
  return Math.min(100, (done / total) * 100);
}

export function dateTime(ms: number | null): string {
  if (!ms) return "--";
  const d = new Date(ms);
  const p = (x: number) => String(x).padStart(2, "0");
  return `${d.getFullYear()}年${p(d.getMonth() + 1)}月${p(d.getDate())}日 ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 相对时间：刚刚 / N 分钟前 / N 小时前 / 昨天 / N 天前，更早显示日期 */
export function ago(ms: number | null, now = Date.now()): string {
  if (!ms) return "--";
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  const days = Math.floor(s / 86400);
  if (days === 1) return "昨天";
  if (days < 7) return `${days} 天前`;
  const d = new Date(ms);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
}

export function extractUrls(text: string): string[] {
  const re = /(?:https?:\/\/|magnet:\?)[^\s"'<>]+/gi;
  return Array.from(new Set(text.match(re) ?? []));
}
