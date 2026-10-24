import {
  ArrowDownToLine,
  CircleCheck,
  ClipboardPaste,
  Gauge,
  HardDrive,
  Inbox,
  Layers,
  Pause,
  Play,
  Plus,
  RotateCcw,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatedNumber, useFlip, useTween } from "./Animated";
import { api, readClipboard, type DiskInfo, type Task } from "../lib/api";
import { bytes, extractUrls, speedParts } from "../lib/format";
import { HISTORY, SAMPLE_MS, filterTasks, useStore, type Filter } from "../store";
import { TaskCard } from "./TaskCard";
import { Segmented } from "./ui";

/**
 * 单调三次插值（Fritsch–Carlson，同 d3 curveMonotoneX）：
 * 曲线一阶导数连续，且不会在数据突变处过冲或出现尖角。
 */
function monotonePath(pts: readonly (readonly [number, number])[]): string {
  const n = pts.length;
  if (n < 2) return "";
  const dx: number[] = [];
  const m: number[] = [];
  for (let i = 0; i < n - 1; i++) {
    dx[i] = pts[i + 1][0] - pts[i][0];
    m[i] = dx[i] ? (pts[i + 1][1] - pts[i][1]) / dx[i] : 0;
  }
  const t: number[] = [m[0]];
  for (let i = 1; i < n - 1; i++) {
    t[i] = m[i - 1] * m[i] <= 0 ? 0 : (3 * (dx[i - 1] + dx[i])) / ((2 * dx[i] + dx[i - 1]) / m[i - 1] + (dx[i] + 2 * dx[i - 1]) / m[i]);
  }
  t[n - 1] = m[n - 2];
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3;
    d += ` C${pts[i][0] + h},${pts[i][1] + t[i] * h} ${pts[i + 1][0] - h},${pts[i + 1][1] - t[i + 1] * h} ${pts[i + 1][0]},${pts[i + 1][1]}`;
  }
  return d;
}

const CHART_W = 300;
const CHART_H = 100;
const CHART_STEP = CHART_W / (HISTORY - 2);

function SpeedChart({ samples }: { samples: { t: number; v: number }[] }) {
  // 纵轴上限平滑过渡，避免峰值变化时整条曲线突变
  const max = useTween(Math.max(...samples.map((s) => s.v), 512 * 1024) * 1.15);
  const w = CHART_W;
  const h = CHART_H;
  // 按采样时间定位：最新点在 x=0，更早的点在左侧（负方向）
  const newest = samples[samples.length - 1].t;
  const pts = samples.map((s) => [((s.t - newest) / SAMPLE_MS) * CHART_STEP, h - (Math.min(s.v, max) / max) * h] as const);
  const gRef = useRef<SVGGElement>(null);
  const newestRef = useRef(newest);
  newestRef.current = newest;

  // 每帧按当前时间平移：曲线以恒定速度左移，与数据到达时机无关。
  // 新点先出现在右侧可视区外，一个采样周期后正好滑到右边缘。
  const place = () => {
    const elapsed = (performance.now() - newestRef.current) / SAMPLE_MS;
    const x = w + CHART_STEP - Math.min(elapsed, 2) * CHART_STEP;
    gRef.current?.setAttribute("transform", `translate(${x} 0)`);
  };
  // 新数据渲染时在绘制前同步平移，路径坐标与平移量同一帧生效，避免错位一帧
  useLayoutEffect(place, [newest]);
  useEffect(() => {
    let raf = 0;
    const frame = () => {
      place();
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const d = monotonePath(pts);
  return (
    <svg className="speed-chart" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
      <g ref={gRef}>
        <path d={`${d} L${pts[pts.length - 1][0]},${h} L${pts[0][0]},${h} Z`} fill="var(--accent)" fillOpacity="0.1" />
        <path d={d} fill="none" stroke="var(--accent)" strokeWidth="1.75" vectorEffect="non-scaling-stroke" strokeLinecap="round" strokeLinejoin="round" />
      </g>
    </svg>
  );
}

function DiskRing({ info }: { info: DiskInfo | null }) {
  const used = info ? 1 - info.free / info.total : 0;
  const r = 26;
  const c = 2 * Math.PI * r;
  return (
    <svg width="64" height="64" viewBox="0 0 64 64" className="disk-ring">
      <circle cx="32" cy="32" r={r} fill="none" stroke="var(--track)" strokeWidth="7" />
      <circle
        cx="32"
        cy="32"
        r={r}
        fill="none"
        stroke={used > 0.9 ? "var(--danger)" : "var(--accent)"}
        strokeWidth="7"
        strokeLinecap="round"
        strokeDasharray={`${c * used} ${c}`}
        style={{ transition: "stroke-dasharray 0.8s ease" }}
        transform="rotate(-90 32 32)"
      />
      <text x="32" y="36" textAnchor="middle" className="ring-text">
        {Math.round(used * 100)}%
      </text>
    </svg>
  );
}

function StatsBento() {
  const tasks = useStore((s) => s.tasks);
  const history = useStore((s) => s.speedHistory);
  const settings = useStore((s) => s.settings);
  const openAdd = useStore((s) => s.openAdd);
  const showToast = useStore((s) => s.showToast);
  const [disk, setDisk] = useState<DiskInfo | null>(null);

  const stats = useMemo(() => {
    const active = tasks.filter((t) => t.status === "active");
    const done = tasks.filter((t) => t.status === "completed");
    return {
      speed: active.reduce((a, t) => a + t.speed, 0),
      active: active.length,
      waiting: tasks.filter((t) => t.status === "waiting").length,
      conns: active.reduce((a, t) => a + t.activeConns, 0),
      done: done.length,
      doneSize: done.reduce((a, t) => a + (t.total ?? 0), 0),
    };
  }, [tasks]);

  const dir = settings?.downloadDir ?? "";
  useEffect(() => {
    if (!dir) return;
    const load = () => api.diskInfo(dir).then(setDisk).catch(() => setDisk(null));
    load();
    const t = setInterval(load, 15000);
    return () => clearInterval(t);
  }, [dir, stats.done]);

  const peak = Math.max(...history.map((s) => s.v));

  const pasteAdd = async () => {
    const urls = extractUrls(await readClipboard());
    if (urls.length) openAdd(urls.join("\n"));
    else {
      showToast("剪贴板中没有链接");
      openAdd();
    }
  };

  return (
    <section className="bento">
      <div className="tile glass span-2 row-2 tile-speed">
        <div className="tile-label">
          <Gauge size={15} /> 实时速度
          {settings && settings.speedLimit > 0 && <span className="pill">限速 {bytes(settings.speedLimit * 1024)}/s</span>}
        </div>
        <div className="speed-big">
          <AnimatedNumber className="num" mode="smooth" value={stats.speed} format={(n) => speedParts(n)[0]} />
          <AnimatedNumber className="unit" mode="smooth" value={stats.speed} format={(n) => speedParts(n)[1]} />
        </div>
        <div className="speed-sub">
          峰值 {bytes(peak)}/s · {stats.conns} 个活动连接
        </div>
        <SpeedChart samples={history} />
      </div>

      <div className="tile glass tile-stat">
        <div className="tile-label">
          <ArrowDownToLine size={15} /> 下载中
        </div>
        <div className="stat-num">{stats.active}</div>
        <div className="stat-sub">{stats.waiting} 个排队</div>
      </div>

      <div className="tile glass tile-stat">
        <div className="tile-label">
          <CircleCheck size={15} /> 已完成
        </div>
        <div className="stat-num">{stats.done}</div>
        <div className="stat-sub">共 {bytes(stats.doneSize)}</div>
      </div>

      <div className="tile glass tile-disk" onClick={() => dir && api.openDir(dir)} title={dir}>
        <div>
          <div className="tile-label">
            <HardDrive size={15} /> 磁盘剩余
          </div>
          <div className="stat-num sm">{bytes(disk?.free ?? null)}</div>
          <div className="stat-sub">共 {bytes(disk?.total ?? null, 0)}</div>
        </div>
        <DiskRing info={disk} />
      </div>

      <button className="tile glass tile-paste" onClick={pasteAdd}>
        <div className="paste-icon">
          <ClipboardPaste size={22} />
        </div>
        <div>
          <div className="paste-title">粘贴链接</div>
          <div className="stat-sub">从剪贴板新建任务</div>
        </div>
      </button>
    </section>
  );
}

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "active", label: "下载中" },
  { value: "waiting", label: "等待中" },
  { value: "completed", label: "已完成" },
  { value: "stopped", label: "已停止" },
];

/** 批量操作栏：有选中任务时替换工具栏右侧按钮 */
function BulkBar({ visible }: { visible: Task[] }) {
  const checked = useStore((s) => s.checked);
  const tasks = useStore((s) => s.tasks);
  const setChecked = useStore((s) => s.setChecked);
  const askRemove = useStore((s) => s.askRemove);
  const chosen = tasks.filter((t) => checked.includes(t.id));
  const size = chosen.reduce((a, t) => a + (t.total ?? 0), 0);
  const allVisible = visible.length > 0 && visible.every((t) => checked.includes(t.id));
  const canStart = chosen.some((t) => t.status === "paused" || t.status === "error");
  const canPause = chosen.some((t) => t.status === "active" || t.status === "waiting");
  const canRedo = chosen.some((t) => t.status !== "active" && t.status !== "waiting");

  return (
    <div className="toolbar-actions bulk-bar">
      <span className="bulk-count">
        已选 <b>{chosen.length}</b> 项{size > 0 ? ` · ${bytes(size)}` : ""}
      </span>
      <button className="btn glass" onClick={() => setChecked(allVisible ? [] : visible.map((t) => t.id))}>
        {allVisible ? "取消全选" : "全选"}
      </button>
      <button className="btn glass" disabled={!canStart} onClick={() => api.resume(checked)}>
        <Play size={15} /> 开始
      </button>
      <button className="btn glass" disabled={!canPause} onClick={() => api.pause(checked)}>
        <Pause size={15} /> 暂停
      </button>
      <button
        className="btn glass"
        disabled={!canRedo}
        onClick={() => chosen.filter((t) => t.status !== "active" && t.status !== "waiting").forEach((t) => api.redownload(t.id))}
      >
        <RotateCcw size={15} /> 重新下载
      </button>
      <button className="btn glass danger" onClick={() => askRemove(checked)}>
        <Trash2 size={15} /> 删除
      </button>
      <button className="icon-btn ghost" title="取消选择 (Esc)" onClick={() => setChecked([])}>
        <X size={17} />
      </button>
    </div>
  );
}

export function TasksView() {
  const tasks = useStore((s) => s.tasks);
  const filter = useStore((s) => s.filter);
  const setFilter = useStore((s) => s.setFilter);
  const openAdd = useStore((s) => s.openAdd);
  const list = filterTasks(tasks, filter);
  const flipRef = useFlip(`${filter}|${list.map((t) => t.id).join(",")}`);
  const checkedCount = useStore((s) => s.checked.length);

  // 切换筛选后，只保留当前列表中可见的选中项
  useEffect(() => {
    const { checked, setChecked, tasks: all } = useStore.getState();
    const visible = new Set(filterTasks(all, filter).map((t) => t.id));
    const kept = checked.filter((id) => visible.has(id));
    if (kept.length !== checked.length) setChecked(kept);
  }, [filter]);
  const counts = useMemo(
    () => Object.fromEntries(FILTERS.map((f) => [f.value, filterTasks(tasks, f.value).length])),
    [tasks],
  );

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>任务</h1>
          <p className="muted">多线程分段加速 · 断点续传 · 动态分段</p>
        </div>
        <button className="btn primary" onClick={() => openAdd()}>
          <Plus size={17} /> 新建任务
        </button>
      </header>

      <StatsBento />

      <div className="toolbar">
        <Segmented value={filter} onChange={setFilter} options={FILTERS.map((f) => ({ ...f, count: counts[f.value] }))} />
        {checkedCount > 0 ? (
          <BulkBar visible={list} />
        ) : (
        <div className="toolbar-actions">
          <button className="btn glass" onClick={() => api.resumeAll()}>
            <Play size={15} /> 全部开始
          </button>
          <button className="btn glass" onClick={() => api.pauseAll()}>
            <Pause size={15} /> 全部暂停
          </button>
          <button className="btn glass" onClick={() => api.clearFinished()} disabled={!counts.completed}>
            <Sparkles size={15} /> 清除已完成
          </button>
        </div>
        )}
      </div>

      {list.length === 0 ? (
        <div className="empty glass">
          {filter === "all" ? <Layers size={40} strokeWidth={1.4} /> : <Inbox size={40} strokeWidth={1.4} />}
          <div className="empty-title">{filter === "all" ? "还没有下载任务" : "这里空空如也"}</div>
          <div className="muted">点击「新建任务」，或直接按 Ctrl+V 粘贴链接</div>
        </div>
      ) : (
        <div className="task-grid">
          {list.map((t) => (
            <div key={t.id} ref={flipRef(t.id)} className="flip-item">
              <TaskCard task={t} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
