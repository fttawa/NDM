import { ExternalLink, FolderOpen, Pause, Play, RotateCw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Task } from "../lib/api";
import { bytes, eta, percent, speed } from "../lib/format";
import { AnimatedNumber } from "./Animated";
import { FileIcon, SegmentBar, StatusChip } from "./TaskCard";
import { Switch } from "./ui";
import { CaptureControls } from "./CaptureControls";

const AUTO_CLOSE_KEY = "ndm.captureAutoClose";
const fmtPct = (n: number) => `${n.toFixed(1)}%`;

function loadAutoClose() {
  try {
    return localStorage.getItem(AUTO_CLOSE_KEY) === "1";
  } catch {
    return false;
  }
}

/** 卡片展开后的下载进度视图（类似 IDM 的下载进度对话框） */
export function CaptureProgress({ initial, onClose }: { initial: Task; onClose: () => void }) {
  const [task, setTask] = useState<Task>(initial);
  const [autoClose, setAutoClose] = useState(loadAutoClose);
  const [confirmCancel, setConfirmCancel] = useState(false);

  // 订阅后端事件：任务列表变化与实时进度
  useEffect(() => {
    const offTasks = api.onTasks((list) => {
      const t = list.find((x) => x.id === initial.id);
      if (!t) return onClose(); // 任务已在别处被删除
      // 列表事件不含实时进度，保留较新的下载量
      setTask((prev) => (t.status === "active" && prev.downloaded > t.downloaded ? { ...t, downloaded: prev.downloaded, segments: prev.segments, speed: prev.speed, activeConns: prev.activeConns } : t));
    });
    const offProgress = api.onProgress((list) => {
      const p = list.find((x) => x.id === initial.id);
      if (p) setTask((prev) => ({ ...prev, ...p }));
      else setTask((prev) => (prev.speed ? { ...prev, speed: 0, activeConns: 0 } : prev));
    });
    // 订阅建立后补拉一次当前状态：小文件可能在订阅完成前就已下载完毕，之后不会再有事件
    Promise.all([offTasks, offProgress])
      .then(() => api.getTasks())
      .then((list) => {
        const t = list.find((x) => x.id === initial.id);
        if (t) setTask((prev) => (t.status === "active" && prev.downloaded > t.downloaded ? prev : t));
      });
    return () => {
      offTasks.then((f) => f());
      offProgress.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [initial.id]);

  // 完成后自动关闭（等进度条完成动画播放完）
  useEffect(() => {
    if (task.status === "completed" && autoClose) {
      const timer = setTimeout(onClose, 1500);
      return () => clearTimeout(timer);
    }
  }, [task.status, autoClose, onClose]);

  const done = task.status === "completed";
  const pct = percent(done ? task.total ?? 0 : task.downloaded, task.total);
  const remaining = task.total ? task.total - task.downloaded : NaN;

  return (
    <div className="cap-progress">
      <div className="cap-head" data-tauri-drag-region>
        <FileIcon name={task.fileName} size={46} />
        <div className="cap-title" data-tauri-drag-region>
          <div className="cap-name static" title={task.fileName} data-tauri-drag-region>
            {task.fileName}
          </div>
          <div className="cap-meta" data-tauri-drag-region>
            <StatusChip status={task.status} />
            <span>{task.total == null ? "大小未知" : bytes(task.total)}</span>
          </div>
        </div>
        <CaptureControls />
        <button className="icon-btn ghost" onClick={onClose} title="隐藏（继续在后台下载）" aria-label="隐藏">
          <X size={17} />
        </button>
      </div>

      <div className="cp-main">
        <div className="cp-top">
          <AnimatedNumber className="cp-pct" value={pct} format={fmtPct} />
          <span className="muted">
            <AnimatedNumber value={done ? task.total ?? 0 : task.downloaded} format={bytes} /> / {bytes(task.total)}
          </span>
        </div>
        {!done && <SegmentBar task={task} tall />}
      </div>

      <div className="cp-stats">
        <div>
          <span>速度</span>
          <b>{task.status === "active" ? <AnimatedNumber mode="smooth" value={task.speed} format={speed} /> : "--"}</b>
        </div>
        <div>
          <span>剩余时间</span>
          <b>{task.status === "active" && task.total ? eta(remaining, task.speed) : done ? "已完成" : "--"}</b>
        </div>
        <div>
          <span>连接</span>
          <b>
            {task.status === "active" ? task.activeConns : 0} / {task.connections}
          </b>
        </div>
        <div>
          <span>断点续传</span>
          <b>{task.total == null ? "检测中" : task.resumable ? "支持" : "不支持"}</b>
        </div>
      </div>

      {task.error ? (
        <div className="cp-error err small" title={task.error}>
          {task.error}
        </div>
      ) : (
        <button className="cp-path" onClick={() => api.reveal(task.id)} title="打开所在文件夹">
          <FolderOpen size={14} />
          <span className="mono">
            {task.saveDir}
            {task.saveDir.endsWith("\\") ? "" : "\\"}
            {task.fileName}
          </span>
        </button>
      )}

      <div className="cap-foot">
        <label className="cp-auto small muted">
          <Switch
            checked={autoClose}
            onChange={(v) => {
              setAutoClose(v);
              try {
                localStorage.setItem(AUTO_CLOSE_KEY, v ? "1" : "0");
              } catch {
                /* 存储不可用时仅本次生效 */
              }
            }}
          />
          完成后关闭
        </label>
        {done ? (
          <>
            <button className="btn glass" onClick={() => api.reveal(task.id)}>
              <FolderOpen size={15} /> 文件夹
            </button>
            <button
              className="btn primary"
              onClick={async () => {
                await api.openFile(task.id);
                onClose();
              }}
            >
              <ExternalLink size={15} /> 打开文件
            </button>
          </>
        ) : confirmCancel ? (
          <>
            <span className="small err">删除任务和已下载部分？</span>
            <button className="btn glass" onClick={() => setConfirmCancel(false)}>
              不了
            </button>
            <button
              className="btn danger-solid"
              onClick={async () => {
                await api.remove([task.id], true);
                onClose();
              }}
            >
              删除
            </button>
          </>
        ) : (
          <>
            <button className="btn glass" onClick={() => setConfirmCancel(true)}>
              取消
            </button>
            {task.status === "active" || task.status === "waiting" ? (
              <button className="btn primary" onClick={() => api.pause([task.id])}>
                <Pause size={15} /> 暂停
              </button>
            ) : (
              <button className="btn primary" onClick={() => api.resume([task.id])}>
                {task.status === "error" ? <RotateCw size={15} /> : <Play size={15} />}
                {task.status === "error" ? "重试" : "继续"}
              </button>
            )}
          </>
        )}
      </div>
    </div>
  );
}
