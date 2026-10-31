import { ArrowDownToLine, ChevronRight, CircleCheck, ClipboardPaste, FolderOpen, Gauge, HardDrive, Plus } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AnimatedNumber, useTween } from "./Animated";
import { api, readClipboard, type DiskInfo } from "../lib/api";
import { FileIcon } from "./TaskCard";
import { ago, bytes, eta, extractUrls, percent, speed, speedParts } from "../lib/format";
import { HISTORY, SAMPLE_MS, useStore } from "../store";

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
  const openDownloads = useStore((s) => s.openDownloads);
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

      <button className="tile glass tile-stat tile-stat-link" onClick={() => openDownloads("active")} title="查看下载中的任务">
        <div className="tile-label">
          <ArrowDownToLine size={15} /> 下载中
        </div>
        <div className="stat-num">{stats.active}</div>
        <div className="stat-sub">{stats.waiting} 个排队</div>
      </button>

      <button className="tile glass tile-stat tile-stat-link" onClick={() => openDownloads("completed")} title="查看已完成的任务">
        <div className="tile-label">
          <CircleCheck size={15} /> 已完成
        </div>
        <div className="stat-num">{stats.done}</div>
        <div className="stat-sub">共 {bytes(stats.doneSize)}</div>
      </button>

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

const LIST_MAX = 5;

/** 首页下方的两个列表：正在下载、最近完成。点击行打开任务详情。 */
function HomeLists() {
  const tasks = useStore((s) => s.tasks);
  const select = useStore((s) => s.select);
  const openDownloads = useStore((s) => s.openDownloads);
  // 「几分钟前」每分钟刷新一次
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);

  const running = tasks
    .filter((t) => t.status === "active" || t.status === "waiting")
    .sort((a, b) => (a.status === b.status ? b.speed - a.speed : a.status === "active" ? -1 : 1));
  const recent = tasks
    .filter((t) => t.status === "completed")
    .sort((a, b) => (b.finishedAt ?? 0) - (a.finishedAt ?? 0));

  return (
    <section className="home-lists">
      <div className="tile glass home-list">
        <div className="home-list-head">
          <div className="tile-label">
            <ArrowDownToLine size={15} /> 正在下载
          </div>
          <button className="home-more" onClick={() => openDownloads("active")}>
            全部 <ChevronRight size={14} />
          </button>
        </div>
        {running.length === 0 ? (
          <div className="home-empty muted small">当前没有进行中的下载</div>
        ) : (
          running.slice(0, LIST_MAX).map((t) => {
            const pct = percent(t.downloaded, t.total);
            return (
              <button key={t.id} className="home-row" onClick={() => select(t.id)}>
                <FileIcon name={t.fileName} size={34} />
                <div className="home-row-main">
                  <div className="home-row-name" title={t.fileName}>{t.fileName}</div>
                  <div className={`home-row-bar ${t.status}`}>
                    <span style={{ width: `${t.total ? pct : 0}%` }} />
                  </div>
                </div>
                <div className="home-row-meta">
                  {t.status === "waiting" ? (
                    <span className="muted">排队中</span>
                  ) : (
                    <>
                      <b>{speed(t.speed)}</b>
                      <span className="muted">{t.total ? `${pct.toFixed(0)}% · ${eta(t.total - t.downloaded, t.speed)}` : bytes(t.downloaded)}</span>
                    </>
                  )}
                </div>
              </button>
            );
          })
        )}
      </div>

      <div className="tile glass home-list">
        <div className="home-list-head">
          <div className="tile-label">
            <CircleCheck size={15} /> 最近完成
          </div>
          <button className="home-more" onClick={() => openDownloads("completed")}>
            全部 <ChevronRight size={14} />
          </button>
        </div>
        {recent.length === 0 ? (
          <div className="home-empty muted small">完成的下载会显示在这里</div>
        ) : (
          recent.slice(0, LIST_MAX).map((t) => (
            <div key={t.id} className="home-row" role="button" tabIndex={0} onClick={() => select(t.id)} onKeyDown={(e) => e.key === "Enter" && select(t.id)}>
              <FileIcon name={t.fileName} size={34} />
              <div className="home-row-main">
                <div className="home-row-name" title={t.fileName}>{t.fileName}</div>
                <div className="muted small">{bytes(t.total)} · {ago(t.finishedAt, now)}</div>
              </div>
              <button
                className="icon-btn ghost"
                title="打开所在文件夹"
                aria-label="打开所在文件夹"
                onClick={(e) => {
                  e.stopPropagation();
                  void api.reveal(t.id);
                }}
              >
                <FolderOpen size={16} />
              </button>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

export function HomeView() {
  const openAdd = useStore((s) => s.openAdd);
  return <div className="view">
    <header className="view-head">
      <h1>首页</h1>
      <button className="btn primary" onClick={() => openAdd()}><Plus size={17} /> 新建任务</button>
    </header>
    <StatsBento />
    <HomeLists />

  </div>;
}
