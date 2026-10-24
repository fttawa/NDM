// OpenList / AList 当前文件夹爬虫
// 以下两个函数通过 chrome.scripting.executeScript 注入页面主环境（world: "MAIN"）执行，
// 因此必须完全自包含（不能引用外部变量），可直接读取页面的配置、登录 token 与文件夹密码。

/** 当前页面是否为 OpenList（或 AList）站点 */
export function detectOpenList() {
  const cfg = window.OPENLIST_CONFIG || window.ALIST;
  return cfg ? { kind: window.OPENLIST_CONFIG ? "OpenList" : "AList" } : null;
}

/**
 * 列出当前文件夹的文件（可递归子文件夹），返回 { items, title, truncated } 或 { error }。
 * 下载链接与官方前端一致：{api}/d{用户根目录}{目录}/{文件名}?sign={签名}
 */
export async function crawlOpenList(recursive) {
  const MAX_FILES = 5000;
  const MAX_DEPTH = 20;
  const CONCURRENCY = 4;
  try {
    const cfg = window.OPENLIST_CONFIG || window.ALIST;
    if (!cfg) return { error: "当前页面不是 OpenList 站点" };

    const trimSlash = (s) => (s || "").replace(/\/+$/, "");
    const join = (...ps) => ps.join("/").replace(/\/{2,}/g, "/");
    let base = trimSlash(cfg.base_path || "");
    if (base && !base.startsWith("/")) base = "/" + base;
    let api = cfg.api && cfg.api !== "/" ? cfg.api : location.origin + base;
    api = trimSlash(api);

    const token = localStorage.getItem("token") || "";
    const cookie = document.cookie.split("; ").find((c) => c.startsWith("browser-password="));
    const password = cookie ? decodeURIComponent(cookie.slice("browser-password=".length)) : "";

    // 浏览器地址即文件路径（去掉站点子路径）
    let path = decodeURIComponent(location.pathname);
    if (base && path.startsWith(base)) path = path.slice(base.length);
    path = "/" + path.replace(/^\/+|\/+$/g, "");
    if (/^\/@(manage|login|s)\b/.test(path)) return { error: "请先打开一个文件夹页面" };

    const headers = { "Content-Type": "application/json;charset=utf-8", Authorization: token };
    const list = async (dir) => {
      const r = await fetch(api + "/api/fs/list", {
        method: "POST",
        headers,
        body: JSON.stringify({ path: dir, password, page: 1, per_page: 0, refresh: false }),
      });
      const j = await r.json();
      if (j.code !== 200) throw new Error(j.message || `HTTP ${j.code}`);
      return j.data?.content || [];
    };

    // 下载链接需要带上当前用户的根目录（访客一般为 /）
    let userBase = "/";
    try {
      const me = await (await fetch(api + "/api/me", { headers: { Authorization: token } })).json();
      if (me.code === 200 && me.data?.base_path) userBase = me.data.base_path;
    } catch {
      /* 取不到时按根目录处理 */
    }
    const encodePath = (p) => p.split("/").map(encodeURIComponent).join("/");

    const items = [];
    let truncated = false;
    // 广度优先，限制并发，避免压垮服务器
    let queue = [{ dir: path, rel: "", depth: 0 }];
    while (queue.length && !truncated) {
      const batch = queue.splice(0, CONCURRENCY);
      const results = await Promise.all(batch.map((q) => list(q.dir).then((objs) => ({ q, objs }))));
      for (const { q, objs } of results) {
        for (const o of objs) {
          if (o.is_dir) {
            if (recursive && q.depth < MAX_DEPTH) {
              queue.push({ dir: join(q.dir, o.name), rel: q.rel ? `${q.rel}/${o.name}` : o.name, depth: q.depth + 1 });
            }
            continue;
          }
          if (items.length >= MAX_FILES) {
            truncated = true;
            break;
          }
          let url = api + "/d" + encodePath(join(userBase, q.dir, o.name));
          if (o.sign) url += "?sign=" + o.sign;
          items.push({ url, name: o.name, size: typeof o.size === "number" ? o.size : null, dir: q.rel });
        }
      }
    }

    const title = path === "/" ? document.title || location.hostname : path.split("/").pop();
    return { items, title, truncated };
  } catch (e) {
    return { error: `读取文件列表失败：${e.message || e}` };
  }
}
