// 后端接口封装；在普通浏览器中运行时使用 mock，便于预览 UI

export type Status = "waiting" | "active" | "paused" | "completed" | "error";

export interface Segment {
  start: number;
  end: number;
  pos: number;
}

export interface Task {
  id: string;
  url: string;
  fileName: string;
  nameLocked: boolean;
  hls?: boolean;
  torrent?: { infoHash: string; files: string[]; version?: number } | null;
  saveDir: string;
  total: number | null;
  downloaded: number;
  status: Status;
  resumable: boolean;
  segments: Segment[];
  connections: number;
  createdAt: number;
  finishedAt: number | null;
  error: string | null;
  mime: string | null;
  speed: number;
  activeConns: number;
  /** 当前超频连接数 */
  boost: number;
  /** 请求延迟（毫秒），0 表示未测得 */
  latencyMs: number;
}

export interface Progress {
  id: string;
  downloaded: number;
  total: number | null;
  speed: number;
  activeConns: number;
  boost: number;
  latencyMs: number;
  segments: Segment[];
}

export interface Settings {
  downloadDir: string;
  maxConcurrent: number;
  connections: number;
  speedLimit: number;
  userAgent: string;
  notify: boolean;
  clipboardWatch: boolean;
  closeToTray: boolean;
  resumeOnStart: boolean;
  glass: "acrylic" | "mica" | "none";
  browserIntegration: boolean;
  captureConfirm: boolean;
  proxyMode: "system" | "none" | "custom";
  proxyUrl: string;
  /** 连接超频：按请求延迟提前补连接，最多加开每任务连接数的一半 */
  overclock: boolean;
}

/** 浏览器扩展转交的下载（需要用户确认） */
export interface CapturePayload {
  url: string;
  fileName: string | null;
  headers: [string, string][];
  fileSize: number | null;
  referrer: string | null;
  hls?: boolean;
}

/** 批量下载（如 OpenList 当前文件夹） */
export interface BatchItem {
  url: string;
  name: string;
  size: number | null;
  /** 相对当前文件夹的子目录，以 / 分隔 */
  dir: string;
}

export interface BatchPayload {
  source: string;
  title: string;
  referrer: string | null;
  headers: [string, string][];
  items: BatchItem[];
  truncated: boolean;
}

export interface ImportPreview {
  ticket: string;
  taskCount: number;
  currentCount: number;
  settingsChanges: (keyof Settings)[];
}

export interface HlsInfo {
  variants: { url: string; label: string; supported: boolean }[];
  segments: number;
  duration: number;
  format: string | null;
}

export interface UpdateInfo {
  version: string;
  available: boolean;
  notes: string;
  url: string;
}

export interface BridgeStatus {
  running: boolean;
  port: number;
  error: string | null;
}

export interface AddOptions {
  saveDir?: string;
  fileName?: string;
  /** 与 urls 一一对应的文件名 */
  fileNames?: string[];
  connections?: number;
  start?: boolean;
  /** 附加请求头（来自浏览器的 Cookie、Referer 等） */
  headers?: [string, string][];
  hls?: boolean;
}

export interface DiskInfo {
  free: number;
  total: number;
}

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Unlisten = () => void;

interface Backend {
  call<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  on<T>(event: string, cb: (payload: T) => void): Promise<Unlisten>;
}

const tauriBackend: Backend = {
  async call(cmd, args) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke(cmd, args);
  },
  async on(event, cb) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen(event, (e) => cb(e.payload as never));
  },
};

const backend: Backend = isTauri ? tauriBackend : createMock();

export const api = {
  getTasks: () => backend.call<Task[]>("get_tasks"),
  getSettings: () => backend.call<Settings>("get_settings"),
  setSettings: (settings: Settings) => backend.call<Settings>("set_settings", { settings }),
  addTasks: (urls: string[], options: AddOptions) => backend.call<Task[]>("add_tasks", { urls, options }),
  inspectHls: (url: string, headers: [string, string][] = []) => backend.call<HlsInfo>("inspect_hls", { url, headers }),
  addTorrentFile: (path: string, options: AddOptions) => backend.call<Task[]>("add_torrent_file", { path, options }),
  pause: (ids: string[]) => backend.call<void>("pause_tasks", { ids }),
  resume: (ids: string[]) => backend.call<void>("resume_tasks", { ids }),
  remove: (ids: string[], deleteFile: boolean) => backend.call<void>("remove_tasks", { ids, deleteFile }),
  pauseAll: () => backend.call<void>("pause_all"),
  resumeAll: () => backend.call<void>("resume_all"),
  clearFinished: () => backend.call<void>("clear_finished"),
  redownload: (id: string) => backend.call<void>("redownload", { id }),
  openFile: (id: string) => backend.call<void>("open_task_file", { id }),
  reveal: (id: string) => backend.call<void>("reveal_task", { id }),
  /** 用系统浏览器打开网址（仅 https） */
  openUrl: (url: string) => (isTauri ? backend.call<void>("open_url", { url }) : Promise.resolve(void window.open(url, "_blank"))),
  openDir: (path: string) => backend.call<void>("open_dir", { path }),
  effectiveGlass: () => backend.call<Settings["glass"]>("effective_glass"),
  onGlass: (cb: (g: Settings["glass"]) => void) => backend.on("ndm://glass", cb),
  bridgeStatus: () => backend.call<BridgeStatus>("bridge_status"),
  exportData: (path: string) => backend.call<void>("export_data", { path }),
  previewImport: (path: string) => backend.call<ImportPreview>("preview_import", { path }),
  importData: (ticket: string) => backend.call<string>("import_data", { ticket }),
  exportDiagnostics: (path: string) => backend.call<void>("export_diagnostics", { path }),
  checkUpdate: () => backend.call<UpdateInfo>("check_update"),
  extensionDir: () => backend.call<string>("extension_dir"),
  getCapture: (id: string) => backend.call<CapturePayload | null>("get_capture", { id }),
  getBatch: (id: string) => backend.call<BatchPayload | null>("get_batch", { id }),
  captureReady: () => backend.call<void>("capture_ready"),
  closeCapture: (id: string) => backend.call<void>("close_capture", { id }),
  testProxy: (settings: Settings) => backend.call<number>("test_proxy", { settings }),
  diskInfo: (path: string) => backend.call<DiskInfo | null>("disk_info", { path }),
  onTasks: (cb: (t: Task[]) => void) => backend.on("ndm://tasks", cb),
  onProgress: (cb: (p: Progress[]) => void) => backend.on("ndm://progress", cb),
};

export async function pickDirectory(defaultPath?: string): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true, defaultPath });
  return typeof r === "string" ? r : null;
}

export async function pickTorrentFile(): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const result = await open({ multiple: false, filters: [{ name: "BitTorrent 种子", extensions: ["torrent"] }] });
  return typeof result === "string" ? result : null;
}

export async function pickExportPath(defaultPath?: string): Promise<string | null> {
  if (!isTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const r = await save({ defaultPath, filters: [{ name: "NDM 数据备份", extensions: ["json"] }] });
  return typeof r === "string" ? r : null;
}

export async function pickImportPath(): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: false, filters: [{ name: "NDM 数据备份", extensions: ["json"] }] });
  return typeof r === "string" ? r : null;
}

export async function readClipboard(): Promise<string> {
  try {
    if (isTauri) {
      const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
      return (await readText()) ?? "";
    }
  } catch {
    /* 剪贴板为空或不可用 */
  }
  return "";
}

export async function writeClipboard(text: string) {
  if (isTauri) {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  } else {
    await navigator.clipboard?.writeText(text);
  }
}

/**
 * 以缓动动画调整当前窗口高度（保持顶部位置不变，向下展开）。
 * 浏览器预览中通过 CSS 变量 --cap-h 模拟。
 */
export async function animateWindowHeight(to: number, duration = 220) {
  // 窗口中由后端线程逐帧调整尺寸（无每帧 IPC 往返，更流畅）
  if (isTauri) return backend.call<void>("animate_height", { height: to, duration });
  const root = document.documentElement;
  const apply = async (h: number) => {
    root.style.setProperty("--cap-h", `${h}px`);
  };
  const startH = document.querySelector<HTMLElement>(".capture")?.offsetHeight ?? window.innerHeight;
  const start = performance.now();
  await new Promise<void>((done) => {
    const frame = async (now: number) => {
      const t = Math.min(1, (now - start) / duration);
      const eased = 1 - Math.pow(1 - t, 3);
      await apply(Math.round(startH + (to - startH) * eased));
      if (t < 1) requestAnimationFrame(frame);
      else done();
    };
    requestAnimationFrame(frame);
  });
}

export async function windowAction(action: "minimize" | "maximize" | "close") {
  if (!isTauri) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const w = getCurrentWindow();
  if (action === "minimize") await w.minimize();
  else if (action === "maximize") await w.toggleMaximize();
  else await w.close();
}

// ---------------- 浏览器预览用的 mock ----------------

function createMock(): Backend {
  const listeners = new Map<string, Set<(p: unknown) => void>>();
  const emit = (ev: string, p: unknown) => listeners.get(ev)?.forEach((cb) => cb(p));
  // 仅浏览器预览：便于在控制台模拟后端事件（如浏览器扩展拦截）
  (window as unknown as { __ndmMock: unknown }).__ndmMock = { emit };
  let settings: Settings = {
    downloadDir: "C:\\Users\\Demo\\Downloads",
    maxConcurrent: 3,
    connections: 8,
    speedLimit: 0,
    userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141 Safari/537.36",
    notify: true,
    clipboardWatch: true,
    closeToTray: true,
    resumeOnStart: true,
    glass: "acrylic",
    browserIntegration: true,
    captureConfirm: true,
    overclock: true,
    proxyMode: "system",
    proxyUrl: "",
  };
  const MB = 1024 * 1024;
  const mk = (name: string, total: number, status: Status, frac: number, conns = 8): Task => {
    const n = status === "completed" ? 1 : conns;
    const size = Math.floor(total / n);
    const segments = Array.from({ length: n }, (_, i) => {
      const start = i * size;
      const end = i === n - 1 ? total - 1 : start + size - 1;
      const f = status === "completed" ? 1 : Math.min(1, frac * (0.6 + Math.random() * 0.8));
      return { start, end, pos: start + Math.floor((end - start + 1) * f) };
    });
    const downloaded = segments.reduce((a, s) => a + s.pos - s.start, 0);
    return {
      id: Math.random().toString(36).slice(2),
      url: `https://mirror.example.com/files/${encodeURIComponent(name)}`,
      fileName: name,
      nameLocked: true,
      saveDir: settings.downloadDir,
      total,
      downloaded,
      status,
      resumable: true,
      segments,
      connections: conns,
      createdAt: Date.now() - Math.random() * 1e8,
      finishedAt: status === "completed" ? Date.now() - Math.random() * 1e7 : null,
      error: status === "error" ? "无法连接服务器" : null,
      mime: null,
      speed: 0,
      activeConns: 0,
      boost: 0,
      latencyMs: 0,
    };
  };
  let tasks: Task[] = [
    mk("ubuntu-24.04.3-desktop-amd64.iso", 6100 * MB, "active", 0.42),
    mk("Blender-4.5.2-windows-x64.msi", 380 * MB, "active", 0.71, 16),
    mk("4K 风景纪录片 第三集.mkv", 2400 * MB, "active", 0.18),
    mk("node-v24.12.0-x64.msi", 31 * MB, "waiting", 0),
    mk("项目资料合集.zip", 920 * MB, "paused", 0.33),
    mk("design-assets.psd", 210 * MB, "error", 0.55),
    mk("Podcast-EP128.mp3", 86 * MB, "completed", 1),
    mk("vscode-setup-1.105.exe", 112 * MB, "completed", 1),
    mk("annual-report-2026.pdf", 14 * MB, "completed", 1),
  ];
  const clone = () => tasks.map((t) => ({ ...t, segments: t.segments.map((s) => ({ ...s })) }));

  setInterval(() => {
    const progress: Progress[] = [];
    for (const t of tasks) {
      if (t.status !== "active" || !t.total) continue;
      let delta = 0;
      t.segments.forEach((s, i) => {
        if (s.pos > s.end) return;
        // 各连接速度不同且缓慢起伏，接近真实网络
        const base = (0.15 + ((i * 37) % 10) / 10) * 0.6 * MB;
        const rate = base * (1 + 0.25 * Math.sin(Date.now() / 3000 + i));
        const step = Math.min(s.end + 1 - s.pos, Math.floor(rate));
        s.pos += step;
        delta += step;
      });
      // 动态分段：空闲连接拆分剩余最多的分段
      const busy = t.segments.filter((s) => s.pos <= s.end).length;
      if (busy < t.connections) {
        let best = -1;
        let rem = 0;
        t.segments.forEach((s, i) => {
          const r = s.end + 1 - s.pos;
          if (r > rem) [best, rem] = [i, r];
        });
        if (best >= 0 && rem >= 512 * 1024) {
          const s = t.segments[best];
          const mid = s.pos + Math.floor(rem / 2);
          t.segments.splice(best + 1, 0, { start: mid, end: s.end, pos: mid });
          s.end = mid - 1;
        }
      }
      t.downloaded = t.segments.reduce((a, s) => a + s.pos - s.start, 0);
      t.speed = delta * 2;
      t.activeConns = t.segments.filter((s) => s.pos <= s.end).length;
      // 模拟超频：尾段较多时临时多开几条连接
      const tail = t.segments.filter((s) => s.pos <= s.end && s.end + 1 - s.pos < 2 * MB).length;
      t.boost = settings.overclock ? Math.min(Math.floor(t.connections / 2), tail) : 0;
      t.activeConns += t.boost;
      t.latencyMs = Math.round(180 + 40 * Math.sin(Date.now() / 5000));
      if (t.downloaded >= t.total) {
        t.status = "completed";
        t.speed = 0;
        t.finishedAt = Date.now();
        const next = tasks.find((x) => x.status === "waiting");
        if (next) next.status = "active";
        emit("ndm://tasks", clone());
      }
      progress.push({
        id: t.id,
        downloaded: t.downloaded,
        total: t.total,
        speed: t.speed,
        activeConns: t.activeConns,
        boost: t.boost,
        latencyMs: t.latencyMs,
        segments: t.segments.map((s) => ({ ...s })),
      });
    }
    emit("ndm://progress", progress);
  }, 500);

  const update = (ids: string[], f: (t: Task) => void) => {
    tasks.filter((t) => ids.includes(t.id)).forEach(f);
    emit("ndm://tasks", clone());
  };

  return {
    async call(cmd, args = {}) {
      const a = args as Record<string, never>;
      switch (cmd) {
        case "get_tasks":
          return clone() as never;
        case "get_settings":
          return { ...settings } as never;
        case "set_settings":
          settings = a.settings;
          return settings as never;
        case "add_tasks": {
          const urls = a.urls as string[];
          const created = urls.map((u) => {
            const name = decodeURIComponent(u.split("?")[0].split("/").filter(Boolean).pop() || "download");
            const t = mk(name, (50 + Math.random() * 800) * MB, "active", 0);
            t.url = u;
            return t;
          });
          tasks = [...created, ...tasks];
          emit("ndm://tasks", clone());
          return created as never;
        }
        case "pause_tasks":
          update(a.ids, (t) => {
            if (t.status === "active" || t.status === "waiting") {
              t.status = "paused";
              t.speed = 0;
            }
          });
          return undefined as never;
        case "resume_tasks":
          update(a.ids, (t) => {
            if (t.status === "paused" || t.status === "error") {
              t.status = "active";
              t.error = null;
            }
          });
          return undefined as never;
        case "pause_all":
          update(tasks.map((t) => t.id), (t) => {
            if (t.status === "active" || t.status === "waiting") t.status = "paused";
          });
          return undefined as never;
        case "resume_all":
          update(tasks.map((t) => t.id), (t) => {
            if (t.status === "paused" || t.status === "error") t.status = "active";
          });
          return undefined as never;
        case "remove_tasks":
          tasks = tasks.filter((t) => !(a.ids as string[]).includes(t.id));
          emit("ndm://tasks", clone());
          return undefined as never;
        case "clear_finished":
          tasks = tasks.filter((t) => t.status !== "completed");
          emit("ndm://tasks", clone());
          return undefined as never;
        case "redownload":
          update([a.id], (t) => {
            t.segments.forEach((s) => (s.pos = s.start));
            t.downloaded = 0;
            t.status = "active";
          });
          return undefined as never;
        case "get_capture":
          return {
            url: "https://objects.example.com/releases/download/v0.2.38/horizon-windows-x64.zip",
            fileName: "horizon-windows-x64.zip",
            headers: [["Referer", "https://github.com/ludovic111/horizon/releases"]],
            fileSize: 28626944,
            referrer: "https://github.com/ludovic111/horizon/releases",
          } as never;
        case "get_batch":
          return {
            source: "openlist",
            title: "电影合集",
            referrer: "https://pan.example.com/电影合集",
            headers: [["Referer", "https://pan.example.com/电影合集"]],
            truncated: false,
            items: [
              { url: "https://pan.example.com/d/a.mkv?sign=x", name: "流浪地球 2.mkv", size: 4.2e9, dir: "" },
              { url: "https://pan.example.com/d/b.mkv?sign=x", name: "封神第一部.mkv", size: 3.1e9, dir: "" },
              { url: "https://pan.example.com/d/c.srt?sign=x", name: "封神第一部.srt", size: 86e3, dir: "" },
              { url: "https://pan.example.com/d/d.mp4?sign=x", name: "花絮 01.mp4", size: 220e6, dir: "花絮" },
              { url: "https://pan.example.com/d/e.mp4?sign=x", name: "花絮 02.mp4", size: 180e6, dir: "花絮" },
              { url: "https://pan.example.com/d/f.jpg?sign=x", name: "poster.jpg", size: 1.2e6, dir: "海报/高清" },
              { url: "https://pan.example.com/d/g.txt?sign=x", name: "说明.txt", size: 2e3, dir: "" },
            ],
          } as never;
        case "capture_ready":
        case "close_capture":
          return undefined as never;
        case "effective_glass":
          return settings.glass as never;
        case "bridge_status":
          return { running: settings.browserIntegration, port: 16800, error: null } as never;
        case "check_update":
          return { version: __APP_VERSION__, available: false, notes: "预览模式", url: "https://github.com/fttawa/NDM/releases" } as never;
        case "extension_dir":
          return "E:\NDM\extension" as never;
        case "test_proxy":
          await new Promise((r) => setTimeout(r, 400));
          return 180 as never;
        case "disk_info":
          return { free: 312 * 1024 * MB, total: 1000 * 1024 * MB } as never;
        default:
          return undefined as never;
      }
    },
    async on(event, cb) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      const set = listeners.get(event)!;
      set.add(cb as (p: unknown) => void);
      return () => set.delete(cb as (p: unknown) => void);
    },
  };
}
