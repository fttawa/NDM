import { Copy, ExternalLink, FolderOpen, Pause, Play, RotateCcw, Trash2, X } from "lucide-react";
import { api, writeClipboard } from "../lib/api";
import { bytes, dateTime, eta, percent, speed } from "../lib/format";
import { useStore } from "../store";
import { AnimatedNumber } from "./Animated";
import { FileIcon, SegmentBar, StatusChip, fileKind } from "./TaskCard";

export function TaskDrawer() {
  const id = useStore((s) => s.selectedId);
  const task = useStore((s) => s.tasks.find((t) => t.id === s.selectedId && !t.torrent));
  const select = useStore((s) => s.select);
  const askRemove = useStore((s) => s.askRemove);
  const showToast = useStore((s) => s.showToast);
  const open = !!id && !!task;

  const pct = task ? percent(task.status === "completed" ? task.total ?? 0 : task.downloaded, task.total) : 0;

  return (
    <>
      <div className={`drawer-mask ${open ? "show" : ""}`} onClick={() => select(null)} />
      <aside className={`drawer glass-strong ${open ? "show" : ""}`}>
        {task && (
          <>
            <div className="drawer-head">
              <FileIcon name={task.fileName} size={52} />
              <div className="drawer-title">
                <div className="name">{task.fileName}</div>
                <div className="sub">
                  <StatusChip status={task.status} /> <span className="muted">{fileKind(task.fileName).label}</span>
                </div>
              </div>
              <button className="icon-btn ghost" onClick={() => select(null)} aria-label="关闭">
                <X size={18} />
              </button>
            </div>

            <div className="drawer-progress tile glass">
              <div className="dp-top">
                <AnimatedNumber className="dp-pct" value={pct} format={(n) => `${n.toFixed(1)}%`} />
                <span className="muted">
                  <AnimatedNumber value={task.status === "completed" ? task.total ?? 0 : task.downloaded} format={bytes} /> / {bytes(task.total)}
                </span>
              </div>
              <SegmentBar task={task} tall />
              <div className="dp-meta">
                <div>
                  <span className="muted">速度</span>
                  <b>{task.status === "active" ? <AnimatedNumber mode="smooth" value={task.speed} format={speed} /> : "--"}</b>
                </div>
                <div>
                  <span className="muted">剩余时间</span>
                  <b>{task.status === "active" && task.total ? eta(task.total - task.downloaded, task.speed) : "--"}</b>
                </div>
                <div>
                  <span className="muted">{task.torrent ? "Peer" : "连接"}</span>
                  <b>
                    {task.status === "active" ? task.activeConns : 0} / {task.connections}
                    {task.status === "active" && task.boost > 0 && <span className="boost">+{task.boost}</span>}
                  </b>
                </div>
                <div>
                  <span className="muted">{task.torrent ? "文件" : "分段"}</span>
                  <b>{task.torrent ? task.torrent.files.length || "解析中" : task.segments.length || 1}</b>
                </div>
              </div>
            </div>

            <dl className="info-list">
              <dt>下载链接</dt>
              <dd className="mono link">
                <span>{task.url}</span>
                <button
                  className="icon-btn ghost"
                  title="复制链接"
                  onClick={async () => {
                    await writeClipboard(task.url);
                    showToast("链接已复制");
                  }}
                >
                  <Copy size={14} />
                </button>
              </dd>
              <dt>保存位置</dt>
              <dd className="mono">{task.saveDir}</dd>
              <dt>断点续传</dt>
              <dd>{task.torrent ? "支持 · 继续时校验已有分片" : task.hls ? "HLS · 暂停后从头下载" : task.total == null ? "未知" : task.resumable ? "支持" : "不支持"}</dd>
              {task.torrent && <><dt>BT 内核</dt><dd>libtorrent · {task.torrent.version === 2 ? "BTv2" : "BTv1 / 混合"} · 完成后停止做种</dd><dt>Info Hash</dt><dd className="mono">{task.torrent.infoHash}</dd></>}
              {task.status === "active" && task.latencyMs > 0 && (
                <>
                  <dt>请求延迟</dt>
                  <dd>{task.latencyMs} ms</dd>
                </>
              )}
              <dt>创建时间</dt>
              <dd>{dateTime(task.createdAt)}</dd>
              {task.finishedAt && (
                <>
                  <dt>完成时间</dt>
                  <dd>{dateTime(task.finishedAt)}</dd>
                </>
              )}
              {task.error && (
                <>
                  <dt>错误信息</dt>
                  <dd className="err">{task.error}</dd>
                </>
              )}
            </dl>
            {task.torrent && task.torrent.files.length > 0 && <details className="field"><summary>种子文件（{task.torrent.files.length}）</summary><ul className="small">{task.torrent.files.map(file => <li key={file} style={{ overflowWrap: "anywhere" }}>{file}</li>)}</ul></details>}

            <div className="drawer-actions">
              {(task.status === "active" || task.status === "waiting") && (
                <button className="btn glass" onClick={() => api.pause([task.id])}>
                  <Pause size={15} /> 暂停
                </button>
              )}
              {(task.status === "paused" || task.status === "error") && (
                <button className="btn primary" onClick={() => api.resume([task.id])}>
                  <Play size={15} /> {task.status === "error" ? "重试" : "继续"}
                </button>
              )}
              {task.status === "completed" && (
                <button className="btn primary" onClick={() => api.openFile(task.id)}>
                  <ExternalLink size={15} /> {task.torrent ? "打开内容文件夹" : "打开文件"}
                </button>
              )}
              <button className="btn glass" onClick={() => api.reveal(task.id)}>
                <FolderOpen size={15} /> 文件夹
              </button>
              {task.status !== "active" && task.status !== "waiting" && (
                <button className="btn glass" onClick={() => api.redownload(task.id)}>
                  <RotateCcw size={15} /> 重新下载
                </button>
              )}
              <button className="btn glass danger" onClick={() => askRemove([task.id])}>
                <Trash2 size={15} /> 删除
              </button>
            </div>
          </>
        )}
      </aside>
    </>
  );
}
