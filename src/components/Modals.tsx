import { FolderOpen, Globe, Link2, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, pickDirectory } from "../lib/api";
import { bytes, extractUrls } from "../lib/format";
import { useStore } from "../store";
import { Slider, Switch } from "./ui";

export function Modal({ open, onClose, children, width = 560 }: { open: boolean; onClose: () => void; children: React.ReactNode; width?: number }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="modal-mask" onMouseDown={onClose}>
      <div className="modal glass-strong" style={{ width }} onMouseDown={(e) => e.stopPropagation()}>
        {children}
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
  const [busy, setBusy] = useState(false);
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
      // 拦截的下载直接聚焦「开始下载」，按回车即可确认
      setTimeout(() => (prefill.headers ? startRef.current : ref.current)?.focus(), 30);
    }
  }, [open, prefill]);

  const urls = useMemo(() => extractUrls(text), [text]);

  const submit = async () => {
    if (!urls.length) return;
    setBusy(true);
    try {
      await api.addTasks(urls, {
        saveDir: dir,
        fileName: urls.length === 1 ? name : undefined,
        connections: conns,
        start,
        headers: prefill.headers,
      });
      useStore.getState().openDownloads();
      showToast(`已添加 ${urls.length} 个任务`);
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
              : "每行一个链接，支持一次粘贴多个"}
          </p>
        </div>
        <button className="icon-btn ghost" onClick={close} aria-label="关闭">
          <X size={18} />
        </button>
      </div>

      <div className="field">
        <textarea
          ref={ref}
          className="input mono"
          rows={captured ? 2 : 5}
          placeholder="https://example.com/file.zip"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
          }}
        />
        <div className="field-hint">{urls.length ? `识别到 ${urls.length} 个链接` : "未识别到链接"}</div>
      </div>

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
            <label>文件名（可选）</label>
            <input className="input" placeholder="留空则自动识别" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
        )}
        <div className="field">
          <label>
            连接数 <b>{conns}</b>
          </label>
          <Slider value={conns} min={1} max={32} onChange={setConns} />
        </div>
        <div className="field row">
          <label>立即开始</label>
          <Switch checked={start} onChange={setStart} />
        </div>
      </div>

      <div className="modal-foot">
        <span className="muted kbd-hint">Ctrl + Enter 提交</span>
        <button className="btn glass" onClick={close}>
          取消
        </button>
        <button ref={startRef} className="btn primary" disabled={!urls.length || busy} onClick={submit}>
          开始下载
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
