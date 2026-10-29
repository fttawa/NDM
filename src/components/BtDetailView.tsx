import { useEffect, useState } from "react";
import { ArrowLeft, Copy, FolderOpen, Pause, Play, RotateCcw, Settings2, Trash2 } from "lucide-react";
import { api, writeClipboard, type BtRuntime } from "../lib/api";
import { bytes, dateTime, eta, percent, speed } from "../lib/format";
import { useStore } from "../store";
import { StatusChip } from "./TaskCard";

const phases: Record<BtRuntime["phase"], string> = { metadata: "正在获取种子元数据", checking: "正在校验已有分片", preparing: "正在准备文件", downloading: "正在下载" };
const trackerStates = { working: "已响应", announcing: "连接中", waiting: "等待公告", error: "连接失败" };

export function BtDetailView() {
  const task = useStore(s => s.tasks.find(t => t.id === s.selectedId));
  const settings = useStore(s => s.settings);
  const setView = useStore(s => s.setView);
  const openDownloads = useStore(s => s.openDownloads);
  const filter = useStore(s => s.filter);
  const askRemove = useStore(s => s.askRemove);
  const toast = useStore(s => s.showToast);
  const [runtime, setRuntime] = useState<BtRuntime | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"overview" | "files" | "trackers">("overview");
  const [filePage, setFilePage] = useState(0);
  useEffect(() => {
    if (task?.id) void api.btMetadata(task.id).catch(e => toast(String(e)));
  }, [task?.id, toast]);
  useEffect(() => {
    let disposed = false; let timer: ReturnType<typeof setTimeout>;
    setRuntime(null); setError("");
    const load = async () => {
      try { const value = await api.btDetails(task!.id); if (!disposed) { setRuntime(value); setError(""); } }
      catch (e) { if (!disposed) setError(String(e)); }
      if (!disposed) timer = setTimeout(load, 1000);
    };
    if (task?.status === "active") void load();
    return () => { disposed = true; clearTimeout(timer); };
  }, [task?.id, task?.status]);
  const action = async (fn: () => Promise<void>) => { try { await fn(); } catch (e) { toast(String(e)); } };
  if (!task?.torrent) return <div className="view"><header className="view-head"><div><h1>任务已移除</h1><p className="muted">可以返回下载列表查看其他任务。</p></div></header><button className="btn glass" onClick={() => openDownloads(filter)}><ArrowLeft size={16} /> 返回下载</button></div>;
  const source = task.torrent, active = task.status === "active", completed = task.status === "completed";
  const live = active ? runtime : null;
  const priv = live?.private ?? source.private;
  const trackers = live?.trackers ?? Array.from(new Set([...(source.trackers ?? []), ...new URL(task.url).searchParams.getAll("tr"), ...(priv ? [] : settings?.bt.trackers ?? [])])).map(url => ({ url, state: "waiting" as const, message: "" }));
  const total = source.files.length, pageCount = Math.max(1, Math.ceil(total / 100)), page = Math.min(filePage, pageCount - 1);
  const pct = percent(completed ? task.total ?? 0 : task.downloaded, task.total);
  return <div className="view bt-detail">
    <button className="bt-back" onClick={() => openDownloads(filter)}><ArrowLeft size={16} /> 返回下载列表</button>
    <header className="view-head"><div className="bt-heading"><div className="bt-eyebrow">BITTORRENT · {source.version === 2 ? "BTv2" : "BTv1 / 混合"}{priv ? " · 私有种子" : ""}</div><h1>{task.fileName}</h1><div className="bt-sub"><StatusChip status={task.status} /><span className="muted small">{active ? live ? phases[live.phase] : "正在启动 BT 内核" : completed ? "下载完成，已停止做种" : "继续后会重新校验已有文件"}</span></div></div></header>
    <div className="bt-actions">
      {(active || task.status === "waiting") && <button className="btn glass" onClick={() => action(() => api.pause([task.id]))}><Pause size={15} /> 暂停</button>}
      {(task.status === "paused" || task.status === "error") && <button className="btn primary" onClick={() => action(() => api.resume([task.id]))}><Play size={15} /> {task.status === "error" ? "重试" : "继续下载"}</button>}
      <button className="btn glass" onClick={() => action(() => api.reveal(task.id))}><FolderOpen size={15} /> 内容文件夹</button>
      <button className="btn glass" onClick={() => setView("bt-settings")}><Settings2 size={15} /> BT 配置</button>
      {!active && task.status !== "waiting" && <button className="btn glass" onClick={() => action(() => api.redownload(task.id))}><RotateCcw size={15} /> 重新下载</button>}
      <button className="btn glass danger" onClick={() => askRemove([task.id])}><Trash2 size={15} /> 删除</button>
    </div>
    {task.error && <p className="err" role="alert">{task.error}</p>}
    {error && <p className="err" role="alert">详情更新失败：{error}</p>}
    <section className="tile glass bt-progress">
      <div className="dp-top"><b className="dp-pct">{pct.toFixed(1)}%</b><span className="muted">{bytes(completed ? task.total ?? task.downloaded : task.downloaded)} / {bytes(task.total)}</span></div>
      <div className="bt-progress-track" role="progressbar" aria-label="BT 下载进度" aria-valuenow={task.total == null ? undefined : pct} aria-valuemin={0} aria-valuemax={100}><span style={{ width: `${pct}%` }} /></div>
      <div className="bt-sub muted small"><span>{active && task.total ? `预计剩余 ${eta(task.total - task.downloaded, task.speed)}` : total ? `${total} 个文件` : "等待种子元数据"}</span><span>{live ? `已取得 ${live.pieces} / ${live.totalPieces} 个分片` : source.pieceCount ? `${source.pieceCount} 个分片 · 每片 ${bytes(source.pieceLength ?? 0)}` : ""}</span></div>
    </section>
    <nav className="settings-tabs" aria-label="BT 任务详情分类">{([['overview','概览'],['files',`文件（${total}）`],['trackers',`Tracker（${trackers.length}）`]] as const).map(([key,label]) => <button key={key} className={`chip-btn ${tab === key ? "active" : ""}`} aria-pressed={tab === key} onClick={() => setTab(key)}>{label}</button>)}</nav>
    {tab === "overview" && <>
      <section className="bt-metrics">
        <div className="tile glass"><span className="muted small">下载速度</span><b>{active ? speed(task.speed) : "--"}</b></div>
        <div className="tile glass"><span className="muted small">上传速度</span><b>{live ? speed(live.uploadSpeed) : "--"}</b><span className="muted small">{live ? `本次上传 ${bytes(live.uploaded)}` : "完成后停止做种"}</span></div>
        <div className="tile glass"><span className="muted small">已连接 Peer</span><b>{live ? `${live.peers} / ${live.maxPeers}` : active ? "连接中" : "--"}</b><span className="muted small">{live ? `其中 ${live.seeds} 个种子 Peer` : "实时连接数"}</span></div>
        <div className="tile glass"><span className="muted small">DHT 节点</span><b>{live ? live.dht ? live.dhtNodes : "已关闭" : "--"}</b><span className="muted small">{priv ? "私有种子禁用 DHT / PEX" : live ? `PEX ${live.pex ? "已开启" : "已关闭"}` : "下载时显示实时节点数"}</span></div>
      </section>
      <section className="tile glass"><dl className="info-list bt-info">
        <dt>Info Hash</dt><dd className="mono bt-copy"><span>{source.infoHash}</span><button className="icon-btn ghost" aria-label="复制 Info Hash" onClick={() => action(async () => { await writeClipboard(source.infoHash); toast("Info Hash 已复制"); })}><Copy size={14} /></button></dd>
        <dt>磁力链接</dt><dd className="mono bt-copy"><span>{task.url}</span><button className="icon-btn ghost" aria-label="复制磁力链接" onClick={() => action(async () => { await writeClipboard(task.url); toast("磁力链接已复制"); })}><Copy size={14} /></button></dd>
        <dt>内容目录</dt><dd className="mono">{task.saveDir}\{task.fileName}</dd>
        <dt>创建时间</dt><dd>{dateTime(task.createdAt)}</dd>
        {task.finishedAt && <><dt>完成时间</dt><dd>{dateTime(task.finishedAt)}</dd></>}
        <dt>连接方式</dt><dd>直接联网 · {priv ? "仅使用种子原有 Tracker" : "种子 Tracker 与补充 Tracker 合并"}</dd>
      </dl></section>
    </>}
    {tab === "files" && <section className="tile glass bt-files">
      <p className="muted small">{total ? "下载种子内的全部文件。进度按已完成的分片统计；暂停或重启后，继续下载时重新校验。" : "获取元数据后显示文件列表。"}</p>
      {source.files.slice(page * 100, (page + 1) * 100).map((file, i) => {
        const index = page * 100 + i, size = source.fileSizes?.[index], done = completed ? size : live?.fileProgress[index];
        return <div className="bt-file-row" key={file}><div className="bt-file-name mono">{file}</div><div className="bt-file-meta muted small"><span>{size == null ? "大小待解析" : bytes(size)}</span><span>{done == null ? "待校验" : `${bytes(done)} · ${size ? Math.min(100, done / size * 100).toFixed(1) : "100"}%`}</span></div></div>;
      })}
      {pageCount > 1 && <div className="bt-pagination"><button className="btn glass" disabled={page === 0} onClick={() => setFilePage(page - 1)}>上一页</button><span className="muted small">{page + 1} / {pageCount}</span><button className="btn glass" disabled={page + 1 >= pageCount} onClick={() => setFilePage(page + 1)}>下一页</button></div>}
    </section>}
    {tab === "trackers" && <section className="tile glass bt-trackers"><p className="muted small">{live ? "来自内核的公告状态。单个 Tracker 失败时仍可通过其他 Tracker 或 DHT 寻找 Peer。" : "以下为配置地址；开始下载后显示实际公告状态。"}</p>{trackers.length ? trackers.map(tracker => <div className="bt-tracker-row" key={tracker.url}><div className="mono">{tracker.url}</div><span className={`small ${live && tracker.state === "error" ? "err" : "muted"}`}>{live ? trackerStates[tracker.state] : active ? "等待内核" : "未运行"}</span>{live && tracker.state === "error" && tracker.message && <p className="muted small">{tracker.message}</p>}</div>) : <p className="muted">没有 Tracker，可在 BT 配置中添加，或通过 DHT 寻找 Peer。</p>}</section>}
  </div>;
}
