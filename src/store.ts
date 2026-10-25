import { create } from "zustand";
import { api, type Progress, type Settings, type Task } from "./lib/api";

/** 新建任务弹窗的预填内容；来自浏览器扩展时带有文件名、大小与请求头 */
export interface AddPrefill {
  text: string;
  fileName?: string;
  headers?: [string, string][];
  fileSize?: number;
  referrer?: string;
}

export type Filter = "all" | "active" | "waiting" | "completed" | "stopped";
export type View = "tasks" | "settings";

export const HISTORY = 60;
/** 速度曲线采样间隔，与图表滚动动画时长一致 */
export const SAMPLE_MS = 500;

interface State {
  tasks: Task[];
  settings: Settings | null;
  /** 总速度采样（t 为 performance.now() 时间戳），图表按时间连续滚动 */
  speedHistory: { t: number; v: number }[];
  filter: Filter;
  view: View;
  addOpen: boolean;
  addPrefill: AddPrefill;
  selectedId: string | null;
  /** 复选框批量选中的任务 */
  checked: string[];
  /** 上一次点选的任务（Shift 区间选择的起点） */
  lastChecked: string | null;
  confirmIds: string[] | null;
  toast: string | null;

  init: () => Promise<void>;
  reload: () => Promise<void>;
  setFilter: (f: Filter) => void;
  setView: (v: View) => void;
  openAdd: (prefill?: string | AddPrefill) => void;
  closeAdd: () => void;
  select: (id: string | null) => void;
  /** 切换选中；shift 为 true 时选中从上一次点选到当前的整段（按当前列表顺序） */
  toggleCheck: (id: string, shift?: boolean) => void;
  setChecked: (ids: string[]) => void;
  askRemove: (ids: string[] | null) => void;
  saveSettings: (patch: Partial<Settings>) => Promise<void>;
  showToast: (msg: string) => void;
}

function mergeProgress(tasks: Task[], progress: Progress[]): Task[] {
  if (progress.length === 0) {
    return tasks.some((t) => t.speed > 0) ? tasks.map((t) => (t.speed ? { ...t, speed: 0, activeConns: 0, boost: 0 } : t)) : tasks;
  }
  const map = new Map(progress.map((p) => [p.id, p]));
  return tasks.map((t) => {
    const p = map.get(t.id);
    if (!p) return t.status === "active" ? t : t.speed ? { ...t, speed: 0 } : t;
    return { ...t, downloaded: p.downloaded, total: p.total, speed: p.speed, activeConns: p.activeConns, boost: p.boost, latencyMs: p.latencyMs, segments: p.segments };
  });
}

let toastTimer: number | undefined;
let initPromise: Promise<void> | null = null;

export const useStore = create<State>((set, get) => ({
  tasks: [],
  settings: null,
  speedHistory: Array.from({ length: HISTORY }, (_, i) => ({ t: performance.now() - (HISTORY - 1 - i) * SAMPLE_MS, v: 0 })),
  filter: "all",
  view: "tasks",
  addOpen: false,
  addPrefill: { text: "" },
  selectedId: null,
  checked: [],
  lastChecked: null,
  confirmIds: null,
  toast: null,

  init() {
    if (initPromise) return initPromise;
    initPromise = (async () => {
      const [tasks, settings] = await Promise.all([api.getTasks(), api.getSettings()]);
      set({ tasks, settings });
      await api.onTasks((tasks) => {
        // 保留运行时进度，避免列表刷新时进度条回跳
        const prev = new Map(get().tasks.map((t) => [t.id, t]));
        // 已删除的任务从选中中移除
        const ids = new Set(tasks.map((t) => t.id));
        const checked = get().checked.filter((id) => ids.has(id));
        if (checked.length !== get().checked.length) set({ checked });
        set({
          tasks: tasks.map((t) => {
            const old = prev.get(t.id);
            return old && t.status === "active" && old.downloaded > t.downloaded
              ? { ...t, downloaded: old.downloaded, segments: old.segments, speed: old.speed }
              : t;
          }),
        });
      });
      await api.onProgress((progress) => {
        set((s) => ({ tasks: mergeProgress(s.tasks, progress) }));
      });
      // 固定节奏采样总速度，曲线才能匀速滚动；
      // 采样值做指数平滑（只依赖历史值，已画出的曲线形状不会再变）
      setInterval(() => {
        const s = get();
        const total = s.tasks.reduce((a, t) => a + (t.status === "active" ? t.speed : 0), 0);
        const prev = s.speedHistory[s.speedHistory.length - 1].v;
        const v = prev + (total - prev) * 0.35;
        set({ speedHistory: [...s.speedHistory.slice(1), { t: performance.now(), v: v < 1 ? 0 : v }] });
      }, SAMPLE_MS);
    })().catch((error) => {
      initPromise = null;
      throw error;
    });
    return initPromise;
  },

  async reload() {
    const [tasks, settings] = await Promise.all([api.getTasks(), api.getSettings()]);
    set({ tasks, settings, checked: [], selectedId: null });
  },

  setFilter: (filter) => set({ filter }),
  setView: (view) => set({ view }),
  openAdd: (prefill = "") => set({ addOpen: true, addPrefill: typeof prefill === "string" ? { text: prefill } : prefill }),
  closeAdd: () => set({ addOpen: false, addPrefill: { text: "" } }),
  select: (selectedId) => set({ selectedId }),
  toggleCheck(id, shift) {
    const { checked, lastChecked, tasks, filter } = get();
    const visible = filterTasks(tasks, filter).map((t) => t.id);
    if (shift && lastChecked && visible.includes(lastChecked)) {
      const [a, b] = [visible.indexOf(lastChecked), visible.indexOf(id)].sort((x, y) => x - y);
      const range = visible.slice(a, b + 1);
      set({ checked: Array.from(new Set([...checked, ...range])), lastChecked: id });
      return;
    }
    set({
      checked: checked.includes(id) ? checked.filter((x) => x !== id) : [...checked, id],
      lastChecked: id,
    });
  },
  setChecked: (ids) => set({ checked: ids, lastChecked: ids.length ? get().lastChecked : null }),
  askRemove: (confirmIds) => set({ confirmIds }),

  async saveSettings(patch) {
    const cur = get().settings;
    if (!cur) return;
    const next = { ...cur, ...patch };
    set({ settings: next });
    try {
      const saved = await api.setSettings(next);
      set({ settings: saved });
    } catch (e) {
      // 后端拒绝（如代理地址无效）时回滚
      set({ settings: cur });
      get().showToast(String(e));
    }
  },

  showToast(msg) {
    set({ toast: msg });
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => set({ toast: null }), 2400);
  },
}));

export function filterTasks(tasks: Task[], f: Filter): Task[] {
  switch (f) {
    case "active":
      return tasks.filter((t) => t.status === "active");
    case "waiting":
      return tasks.filter((t) => t.status === "waiting");
    case "completed":
      return tasks.filter((t) => t.status === "completed");
    case "stopped":
      return tasks.filter((t) => t.status === "paused" || t.status === "error");
    default:
      return tasks;
  }
}
