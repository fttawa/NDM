import { FolderOpen, Globe, Link2, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, pickDirectory, pickTorrentFile, type HlsInfo } from "../lib/api";
import { bytes, extractUrls } from "../lib/format";
import { useStore } from "../store";
import { Slider, Switch } from "./ui";
import { useLatest, usePresence } from "./Animated";

export function Modal({ open, onClose, children, width = 560 }: { open: boolean; onClose: () => void; children: React.ReactNode; width?: number }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  const { mounted, closing } = usePresence(open, 180);
  // 退场时数据可能已清空，沿用关闭前的内容
  const content = useLatest(children, open);
  if (!mounted) return null;
  return (
    <div className={`modal-mask ${closing ? "closing" : ""}`} onMouseDown={closing ? undefined : onClose}>
      <div className="modal glass-strong" style={{ width }} onMouseDown={(e) => e.stopPropagation()}>
        {content}
      </div>
    </div>
  );
}

function hostOf(url: string) {
  try {
    return new URL(url.trim()).hostname;
  } catch {
    return "";
  }
}

export function AddTaskModal() {
  const open = useStore((s) => s.addOpen);
  const prefill = useStore((s) => s.addPrefill);
  const close = useStore((s) => s.closeAdd);
  const showToast = useStore((s) => s.showToast);
  const [text, setText] = useState("");
  const [dir, setDir] = useState("");
  const [name, setName] = useState("");
  const [conns, setConns] = useState(8);
  const [start, setStart] = useState(true);
  const [scheduled, setScheduled] = useState(false);
  const [scheduledAt, setScheduledAt] = useState("");
  const [busy, setBusy] = useState(false);
  const [hls, setHls] = useState(false);
  const [torrentPath, setTorrentPath] = useState("");
  const [hlsInfo, setHlsInfo] = useState<HlsInfo | null>(null);
  const [hlsVariant, setHlsVariant] = useState("");
  const [hlsError, setHlsError] = useState("");
  const [inspecting, setInspecting] = useState(false);
  const inspectSequence = useRef(0);
  const ref = useRef<HTMLTextAreaElement>(null);
  const startRef = useRef<HTMLButtonElement>(null);
  // 来自浏览器扩展的下载：带请求头，弹窗以确认为主
  const captured = !!prefill.headers;

  // 仅在打开时初始化表单，避免设置变化时清空输入
  useEffect(() => {
    if (open) {
      const s = useStore.getState().settings;
      setText(prefill.text);
      setDir(s?.downloadDir ?? "");
      setName(prefill.fileName ?? "");
      setConns(s?.connections ?? 8);
      setStart(true);
      setScheduled(false); setScheduledAt("");
      setHls(!!prefill.hls);
      setTorrentPath("");
      // 拦截的下载直接聚焦「开始下载」，按回车即可确认
      setTimeout(() => (prefill.headers ? startRef.current : ref.current)?.focus(), 30);
    }
  }, [open, prefill]);

  const urls = useMemo(() => extractUrls(text), [text]);
  const btMode = !!torrentPath || urls.some(url => url.toLowerCase().startsWith("magnet:"));
  const autoHls = urls.length === 1 && (() => { try { return new URL(urls[0]).pathname.toLowerCase().endsWith(".m3u8"); } catch { return false; } })();
  const hlsMode = !btMode && urls.length === 1 && (hls || autoHls);

  useEffect(() => {
    inspectSequence.current++;
    setHlsInfo(null); setHlsVariant(""); setHlsError(""); setInspecting(false);
  }, [text, hls, open, prefill]);

  const inspect = async () => {
    const sequence = ++inspectSequence.current;
    setInspecting(true); setHlsError(""); setHlsInfo(null); setHlsVariant("");
    try {
      const info = await api.inspectHls(urls[0], prefill.headers);
      if (sequence === inspectSequence.current) setHlsInfo(info);
    } catch (error) { if (sequence === inspectSequence.current) setHlsError(String(error)); }
    finally { if (sequence === inspectSequence.current) setInspecting(false); }
  };

  const submit = async () => {
    if ((!urls.length && !torrentPath) || busy || inspecting) return;
    setBusy(true);
    try {
      const at = scheduled ? new Date(scheduledAt).getTime() : null;
      if (scheduled && (!at || !Number.isFinite(at) || at <= Date.now())) throw new Error("请选择未来的开始时间");
      const target = hlsMode && hlsVariant ? hlsVariant : urls[0];
      const headers = hlsMode && new URL(target).origin !== new URL(urls[0]).origin
        ? prefill.headers?.filter(([name]) => !["cookie", "authorization", "origin"].includes(name.toLowerCase()))
        : prefill.headers;
      const options = {
        saveDir: dir,
        fileName: torrentPath || urls.length === 1 ? name : undefined,
        connections: conns,
        start,
        scheduledAt: at,
        headers,
        hls: hlsMode,
      };
      if (torrentPath) await api.addTorrentFile(torrentPath, options);
      else await api.addTasks(hlsMode ? [target] : urls, options);
      useStore.getState().openDownloads();
      showToast(`已添加 ${torrentPath ? 1 : urls.length} 个任务`);
      close();
    } catch (e) {
      showToast(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal open={open} onClose={close}>
      <div className="modal-head">
        <div className="modal-icon">{captured ? <Globe size={18} /> : <Link2 size={18} />}</div>
        <div>
          <h2>{captured ? "来自浏览器的下载" : "新建下载任务"}</h2>
          <p className="muted ellipsis">
            {captured
              ? [hostOf(prefill.referrer || prefill.text), prefill.fileSize ? bytes(prefill.fileSize) : null].filter(Boolean).join(" · ")
              : "粘贴下载链接、磁力链接，或打开种子文件"}
          </p>
        </div>
        <button className="icon-btn ghost" onClick={close} aria-label="关闭">
          <X size={18} />
        </button>
      </div>

      {!captured && <div className="field">
        <button className="btn glass" disabled={busy} onClick={async () => {
          try { const path = await pickTorrentFile(); if (path) { setTorrentPath(path); setText(""); setName(""); } }
          catch (error) { showToast(String(error)); }
        }}><FolderOpen size={15} /> 打开种子文件</button>
        {torrentPath && <div className="set-row"><span className="muted small ellipsis" title={torrentPath}>{torrentPath.split(/[\\/]/).pop()}</span><button className="icon-btn ghost" aria-label="移除种子文件" onClick={() => setTorrentPath("")}><X size={14} /></button></div>}
      </div>}
      {!torrentPath && <div className="field">
        <textarea
          ref={ref}
          className="input mono"
          rows={captured ? 2 : 5}
          placeholder={"https://example.com/file.zip\nmagnet:?xt=urn:btih:…"}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
          }}
        />
        <div className="field-hint">{urls.length ? `识别到 ${urls.length} 个链接` : "未识别到链接"}</div>
      </div>}

      {btMode && <p className="muted small field">BT 内容保存到独立文件夹，支持分片校验和续传，完成后自动停止做种。可在 BT 配置中设置 SOCKS5 代理。</p>}
      {!btMode && urls.length === 1 && <div className="field hls-options">
        <label className="set-row"><span>HLS / m3u8 视频</span>{autoHls ? <span className="pill">已识别</span> : <Switch checked={hlsMode} onChange={setHls} />}</label>
        {hlsMode && <>
          <div className="set-row"><span className="muted small">默认选择最高可用清晰度</span><button className="btn glass" disabled={inspecting || busy} onClick={inspect}>{inspecting ? "解析中…" : "解析清晰度"}</button></div>
          {hlsError && <p className="err small" role="alert">{hlsError}</p>}
          {!!hlsInfo?.variants.length && <select className="input" aria-label="视频清晰度" value={hlsVariant} onChange={event => setHlsVariant(event.target.value)}>
            <option value="">自动选择最高可用清晰度</option>
            {hlsInfo.variants.map((variant, index) => <option key={`${index}-${variant.url}`} value={variant.url} disabled={!variant.supported}>{variant.label}</option>)}
          </select>}
          {hlsInfo && !hlsInfo.variants.length && <span className="muted small">{hlsInfo.segments} 个分片 · {Math.round(hlsInfo.duration)} 秒 · 保存为 {hlsInfo.format?.toUpperCase()}</span>}
          <p className="muted small">支持未加密点播流，按顺序合并为 TS 或 MP4。暂停后重新开始；直播、加密与独立音视频轨暂不支持。</p>
        </>}
      </div>}

      <div className="field-grid">
        <div className="field span-2">
          <label>保存到</label>
          <div className="input-group">
            <input className="input" value={dir} onChange={(e) => setDir(e.target.value)} />
            <button
              className="btn glass"
              onClick={async () => {
                const d = await pickDirectory(dir);
                if (d) setDir(d);
              }}
            >
              <FolderOpen size={15} /> 浏览
            </button>
          </div>
        </div>
        {urls.length <= 1 && (
          <div className="field span-2">
            <label>{btMode ? "任务文件夹名（可选）" : "文件名（可选）"}</label>
            <input className="input" placeholder="留空则自动识别" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
        )}
        {!hlsMode && !btMode && <div className="field">
          <label>
            连接数 <b>{conns}</b>
          </label>
          <Slider value={conns} min={1} max={32} onChange={setConns} />
        </div>}
        <label className="field"><span>开始方式</span><select className="input" value={scheduled ? "scheduled" : start ? "now" : "paused"} onChange={e => { setScheduled(e.target.value === "scheduled"); setStart(e.target.value !== "paused"); }}><option value="now">立即开始</option><option value="scheduled">定时开始</option><option value="paused">暂不开始</option></select></label>
        {scheduled && <label className="field span-2"><span>计划开始时间</span><input className="input" type="datetime-local" required value={scheduledAt} onChange={e => setScheduledAt(e.target.value)} /></label>}
      </div>

      <div className="modal-foot">
        <span className="muted kbd-hint">Ctrl + Enter 提交</span>
        <button className="btn glass" onClick={close}>
          取消
        </button>
        <button ref={startRef} className="btn primary" disabled={(!urls.length && !torrentPath) || busy || inspecting} onClick={submit}>
          {scheduled ? "添加计划任务" : start ? "开始下载" : "添加任务"}
        </button>
      </div>
    </Modal>
  );
}

export function ConfirmRemoveModal() {
  const ids = useStore((s) => s.confirmIds);
  const ask = useStore((s) => s.askRemove);
  const select = useStore((s) => s.select);
  const tasks = useStore((s) => s.tasks);
  const [del, setDel] = useState(false);
  const close = () => ask(null);
  const names = tasks.filter((t) => ids?.includes(t.id)).map((t) => t.fileName);

  useEffect(() => setDel(false), [ids]);

  return (
    <Modal open={!!ids} onClose={close} width={440}>
      <div className="modal-head">
        <div className="modal-icon danger">
          <Trash2 size={18} />
        </div>
        <div>
          <h2>删除任务</h2>
          <p className="muted ellipsis">{names.length === 1 ? names[0] : `${names.length} 个任务`}</p>
        </div>
      </div>
      <label className="check-row">
        <Switch checked={del} onChange={setDel} />
        <span>同时删除已下载的文件</span>
      </label>
      <div className="modal-foot">
        <button className="btn glass" onClick={close}>
          取消
        </button>
        <button
          className="btn danger-solid"
          onClick={async () => {
            if (ids) await api.remove(ids, del);
            select(null);
            close();
          }}
        >
          删除
        </button>
      </div>
    </Modal>
  );
}

export function RefreshLinkModal() {
  const id = useStore(s => s.refreshId);
  const task = useStore(s => s.tasks.find(t => t.id === s.refreshId));
  const closeModal = useStore(s => s.refreshLink);
  const toast = useStore(s => s.showToast);
  const [url, setUrl] = useState("");
  const [restart, setRestart] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => { setUrl(""); setRestart(false); setError(""); }, [id]);
  const close = () => { if (!busy) closeModal(null); };
  const allowed = !!task && !task.hls && !task.torrent && ["paused", "error"].includes(task.status);
  return <Modal open={!!id} onClose={close}>
    <form onSubmit={async event => {
      event.preventDefault(); if (!task || !allowed || busy || !url.trim()) return;
      setBusy(true); setError("");
      try {
        await api.refreshLink(task.id, url, restart);
        toast(restart ? "链接已更新，进度已重置，点击继续开始下载" : "链接已更新，已有分段已保留，点击继续下载");
        closeModal(null);
      } catch (e) { setError(String(e)); } finally { setBusy(false); }
    }}>
      <div className="modal-head"><div className="modal-icon"><Link2 size={18} /></div><div><h2>更新下载链接</h2><p className="muted ellipsis">{task?.fileName ?? "任务已移除"}</p></div><button type="button" className="icon-btn ghost" disabled={busy} onClick={close} aria-label="关闭"><X size={18} /></button></div>
      <label className="field"><span>新的下载链接</span><input className="input mono" type="url" required autoFocus disabled={busy} value={url} placeholder="https://…" onChange={e => { setUrl(e.target.value); setError(""); }} /></label>
      <p className="muted small field">验证文件大小与 ETag / 修改时间后保留已有分段、文件名和保存目录。新链接属于其他站点时，不会转发旧站点的登录信息。</p>
      <label className="check-row"><Switch label="从头下载，清除已有进度" checked={restart} disabled={busy} onChange={setRestart} /><span>从头下载，清除已有进度</span></label>
      {!allowed && <p className="err" role="alert">请先暂停普通下载任务；已完成、HLS 和 BT 任务不支持更新链接。</p>}
      {error && <p className="err" role="alert">{error}</p>}
      <div className="modal-foot"><button type="button" className="btn glass" disabled={busy} onClick={close}>取消</button><button type="submit" className="btn primary" disabled={busy || !allowed || !url.trim()}>{busy ? "验证中…" : "验证并更新"}</button></div>
    </form>
  </Modal>;
}
