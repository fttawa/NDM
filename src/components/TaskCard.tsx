import {
  Check,
  Disc3,
  File,
  FileArchive,
  FileText,
  Film,
  FolderOpen,
  Image,
  Music,
  Package,
  Pause,
  Play,
  RotateCw,
  Trash2,
} from "lucide-react";
import { memo, useRef } from "react";
import { api, type Segment, type Status, type Task } from "../lib/api";
import { bytes, eta, percent, speed } from "../lib/format";
import { useStore } from "../store";
import { AnimatedNumber } from "./Animated";
import { IconButton } from "./ui";

const KINDS: { ext: string[]; icon: typeof File; color: string; label: string }[] = [
  { ext: ["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts"], icon: Film, color: "#b4b4b8", label: "视频" },
  { ext: ["mp3", "flac", "wav", "aac", "ogg", "m4a", "ape", "opus"], icon: Music, color: "#b4b4b8", label: "音频" },
  { ext: ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "psd", "heic", "avif"], icon: Image, color: "#b4b4b8", label: "图片" },
  { ext: ["zip", "rar", "7z", "tar", "gz", "xz", "bz2", "zst"], icon: FileArchive, color: "#b4b4b8", label: "压缩包" },
  { ext: ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "epub"], icon: FileText, color: "#b4b4b8", label: "文档" },
  { ext: ["exe", "msi", "dmg", "pkg", "deb", "rpm", "apk", "appimage", "msix"], icon: Package, color: "#b4b4b8", label: "程序" },
  { ext: ["iso", "img", "vhd", "vhdx"], icon: Disc3, color: "#b4b4b8", label: "镜像" },
];

export function fileKind(name: string) {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return KINDS.find((k) => k.ext.includes(ext)) ?? { icon: File, color: "#b4b4b8", label: "文件", ext: [] };
}

export function FileIcon({ name, size = 44 }: { name: string; size?: number }) {
  const k = fileKind(name);
  const Icon = k.icon;
  return (
    <div className="file-icon" style={{ width: size, height: size, ["--c" as string]: k.color }}>
      <Icon size={size * 0.48} strokeWidth={1.9} />
    </div>
  );
}

const STATUS_LABEL: Record<Status, string> = {
  active: "下载中",
  waiting: "等待中",
  paused: "已暂停",
  completed: "已完成",
  error: "出错",
};

export function StatusChip({ status }: { status: Status }) {
  return <span className={`chip chip-${status}`}>{STATUS_LABEL[status]}</span>;
}

/**
 * IDM 风格的分段进度条。
 * 每段的轨道从分段起点延伸到进度条末尾（起点不变，拆分时轨道不跳动），
 * 填充用 transform: scaleX 做连续过渡，由 GPU 合成。
 */
function SegFill({ start, done, total, fresh }: { start: number; done: number; total: number; fresh?: boolean }) {
  const span = total - start;
  const scale = span > 0 ? Math.max(0, Math.min(1, done / span)) : 0;
  return (
    <div className={`seg ${fresh ? "fresh" : ""}`} style={{ left: `${(start / total) * 100}%`, width: `${(span / total) * 100}%` }}>
      <div className="seg-fill" style={{ transform: `scaleX(${scale})` }} />
    </div>
  );
}

export function SegmentBar({ task, tall }: { task: Task; tall?: boolean }) {
  const { total, segments, status, downloaded } = task;
  // 只在「未完成 → 已完成」的那一刻播放完成动画；本来就已完成的任务直接显示满条
  const prevStatus = useRef<Status | null>(null);
  const playDone = useRef(false);
  if (status !== "completed") playDone.current = false;
  else if (prevStatus.current !== null && prevStatus.current !== "completed") playDone.current = true;
  prevStatus.current = status;

  const cls = `segbar ${tall ? "tall" : ""} st-${status}`;
  if (!total) {
    return <div className={cls}>{status === "active" && <div className="seg-indeterminate" />}</div>;
  }
  const completed = status === "completed";
  // 始终按分段渲染（无分段时视为一段），完成时各段在原位置平滑填满，而不是整体替换
  const items: Segment[] = segments.length ? segments : [{ start: 0, end: total - 1, pos: downloaded }];
  return (
    <div className={cls}>
      {items.map((s, i) => (
        // 以起点为 key：动态拆分插入新段时，已有分段不会被重新挂载
        <SegFill
          key={s.start}
          start={s.start}
          done={completed ? s.end + 1 - s.start : Math.max(0, Math.min(s.pos, s.end + 1) - s.start)}
          total={total}
          fresh={!completed && i > 0 && s.pos === s.start}
        />
      ))}
      {tall &&
        items.slice(1).map((s) => (
          <div key={`d${s.start}`} className="seg-divider" style={{ left: `${(s.start / total) * 100}%` }} />
        ))}
      {completed && (
        // 完成：一条线从中间展开铺满进度条（盖住分段间的缝隙），高进度条中间再画一个对勾
        <div className={`seg-done ${playDone.current ? "play" : ""}`}>
          {tall && (
            <svg className="seg-check" viewBox="0 0 24 24" aria-hidden>
              <path d="M6 12.5l4 4L18 8.5" />
            </svg>
          )}
        </div>
      )}
    </div>
  );
}

const fmtPct = (n: number) => `${n.toFixed(1)}%`;

function TaskCardInner({ task }: { task: Task }) {
  const select = useStore((s) => s.select);
  const askRemove = useStore((s) => s.askRemove);
  const toggleCheck = useStore((s) => s.toggleCheck);
  const checked = useStore((s) => s.checked.includes(task.id));
  // 有任意选中时进入选择模式：所有卡片显示复选框，点击卡片即勾选
  const selecting = useStore((s) => s.checked.length > 0);
  const pct = percent(task.downloaded, task.total);
  const remaining = task.total ? task.total - task.downloaded : NaN;
  const canPause = task.status === "active" || task.status === "waiting";
  const canResume = task.status === "paused" || task.status === "error";

  const stop = (fn: () => void) => (e: React.MouseEvent) => {
    e.stopPropagation();
    fn();
  };

  return (
    <div
      className={`task-card glass st-${task.status} ${checked ? "checked" : ""} ${selecting ? "selecting" : ""}`}
      onClick={(e) => (selecting ? toggleCheck(task.id, e.shiftKey) : select(task.id))}
    >
      <div className="task-head">
        <div className="task-icon">
          <FileIcon name={task.fileName} />
          <button
            type="button"
            className="task-check"
            role="checkbox"
            aria-checked={checked}
            aria-label={checked ? "取消选择" : "选择"}
            onClick={(e) => {
              e.stopPropagation();
              toggleCheck(task.id, e.shiftKey);
            }}
          >
            <Check size={18} strokeWidth={2.6} />
          </button>
        </div>
        <div className="task-title">
          <div className="name" title={task.fileName}>
            {task.fileName}
          </div>
          <div className="sub">
            {task.hls && <span className="pill">HLS</span>}
            {task.torrent && <span className="pill">{task.torrent.version === 2 ? "BTv2" : "BT"}</span>}
            {task.status === "completed" ? (
              bytes(task.total)
            ) : (
              <>
                <AnimatedNumber value={task.downloaded} format={bytes} /> / {bytes(task.total)}
                {task.total ? <AnimatedNumber className="pct" value={pct} format={fmtPct} /> : null}
              </>
            )}
          </div>
        </div>
        <StatusChip status={task.status} />
      </div>

      <SegmentBar task={task} />

      <div className="task-foot">
        <div className="meta">
          {task.status === "active" && (
            <>
              <AnimatedNumber className="speed" mode="smooth" value={task.speed} format={speed} />
              <span>剩余 {eta(remaining, task.speed)}</span>
              <span>
                {task.activeConns} {task.torrent ? "Peer" : "连接"}{task.boost > 0 && <span className="boost">超频 +{task.boost}</span>}
              </span>
            </>
          )}
          {task.status === "waiting" && <span>排队等待中…</span>}
          {task.torrent && task.status === "active" && task.total == null && <span>正在解析磁力元数据…</span>}
          {task.torrent && task.status === "active" && task.total != null && task.activeConns === 0 && <span>校验文件 / 寻找 Peer…</span>}
          {task.status === "paused" && <span>{task.hls ? "HLS · 继续时从头下载" : task.resumable ? "可断点续传" : "不支持续传，将重新下载"}</span>}
          {task.status === "error" && <span className="err" title={task.error ?? ""}>{task.error}</span>}
          {task.status === "completed" && <span>保存于 {task.saveDir}</span>}
        </div>
        <div className="actions">
          {canPause && (
            <IconButton title="暂停" onClick={stop(() => api.pause([task.id]))}>
              <Pause size={16} />
            </IconButton>
          )}
          {canResume && (
            <IconButton title={task.status === "error" ? "重试" : "继续"} onClick={stop(() => api.resume([task.id]))}>
              {task.status === "error" ? <RotateCw size={16} /> : <Play size={16} />}
            </IconButton>
          )}
          <IconButton title="打开所在文件夹" onClick={stop(() => api.reveal(task.id))}>
            <FolderOpen size={16} />
          </IconButton>
          <IconButton title="删除" variant="danger" onClick={stop(() => askRemove([task.id]))}>
            <Trash2 size={16} />
          </IconButton>
        </div>
      </div>
    </div>
  );
}

export const TaskCard = memo(TaskCardInner);
