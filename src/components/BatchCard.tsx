import { FolderDown, FolderOpen, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, pickDirectory, type BatchPayload } from "../lib/api";
import { bytes } from "../lib/format";
import { FileIcon } from "./TaskCard";
import { Switch } from "./ui";
import { CaptureControls } from "./CaptureControls";

/** Windows 目录名非法字符替换为 _，并去掉结尾的点和空格 */
function safeSegment(s: string) {
  return s.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").replace(/[. ]+$/, "").trim() || "_";
}

function joinPath(base: string, ...parts: string[]) {
  const sep = base.includes("/") && !base.includes("\\") ? "/" : "\\";
  const segs = parts.flatMap((p) => p.split("/")).filter(Boolean).map(safeSegment);
  return [base.replace(/[\\/]+$/, ""), ...segs].join(sep);
}

function hostOf(url: string | null) {
  try {
    return url ? new URL(url).hostname : "";
  } catch {
    return "";
  }
}

/** 批量下载卡片：浏览器扩展爬取的文件清单（如 OpenList 当前文件夹），勾选后批量添加 */
export function BatchCard({ id }: { id: string }) {
  const [batch, setBatch] = useState<BatchPayload | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [dir, setDir] = useState("");
  const [newFolder, setNewFolder] = useState(true);
  const [free, setFree] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const startRef = useRef<HTMLButtonElement>(null);

  const close = () => api.closeCapture(id);

  useEffect(() => {
    Promise.all([api.getBatch(id), api.getSettings()]).then(([b, s]) => {
      if (!b) return close();
      document.documentElement.dataset.glass = s.glass;
      setBatch(b);
      setSelected(new Set(b.items.map((_, i) => i)));
      setDir(s.downloadDir);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  useEffect(() => {
    if (!batch) return;
    api.captureReady().finally(() => startRef.current?.focus());
  }, [batch]);

  useEffect(() => {
    if (!dir) return;
    api.diskInfo(dir).then((d) => setFree(d?.free ?? null)).catch(() => setFree(null));
  }, [dir]);

  const chosen = useMemo(() => (batch ? batch.items.filter((_, i) => selected.has(i)) : []), [batch, selected]);
  const chosenSize = chosen.reduce((a, it) => a + (it.size ?? 0), 0);
  const allSelected = !!batch && selected.size === batch.items.length;

  const toggle = (i: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });

  const submit = async (start: boolean) => {
    if (!batch || !chosen.length || busy) return;
    setBusy(true);
    setError(null);
    try {
      // 按目标目录分组（保留子目录结构），每组一次添加
      const root = newFolder ? joinPath(dir, batch.title) : dir;
      const groups = new Map<string, { urls: string[]; names: string[] }>();
      for (const it of chosen) {
        const target = it.dir ? joinPath(root, it.dir) : root;
        const g = groups.get(target) ?? { urls: [], names: [] };
        g.urls.push(it.url);
        g.names.push(it.name);
        groups.set(target, g);
      }
      for (const [saveDir, g] of groups) {
        await api.addTasks(g.urls, { saveDir, fileNames: g.names, start, headers: batch.headers });
      }
      close();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!batch) return <div className="capture batch" />;
  const tooSmall = free != null && chosenSize > free;

  return (
    <div className="capture batch" data-tauri-drag-region>
      <div className="cap-head" data-tauri-drag-region>
        <div className="file-icon" style={{ width: 46, height: 46, ["--c" as string]: "#b4b4b8" }}>
          <FolderDown size={22} strokeWidth={1.9} />
        </div>
        <div className="cap-title" data-tauri-drag-region>
          <div className="cap-name static" title={batch.title} data-tauri-drag-region>
            {batch.title}
          </div>
          <div className="cap-meta" data-tauri-drag-region>
            <span>{batch.source === "openlist" ? "OpenList" : "批量下载"}</span>
            <span>{hostOf(batch.referrer)}</span>
            <span>
              {batch.items.length} 个文件{batch.truncated ? "（已截断）" : ""}
            </span>
          </div>
        </div>
        <CaptureControls />
        <button className="icon-btn ghost" onClick={close} aria-label="取消">
          <X size={17} />
        </button>
      </div>

      <div className="batch-list">
        <label className="batch-row head">
          <input
            type="checkbox"
            checked={allSelected}
            ref={(el) => {
              if (el) el.indeterminate = selected.size > 0 && !allSelected;
            }}
            onChange={() => setSelected(allSelected ? new Set() : new Set(batch.items.map((_, i) => i)))}
          />
          <span className="batch-name">全选</span>
          <span className="batch-size">大小</span>
        </label>
        <div className="batch-scroll">
          {batch.items.map((it, i) => (
            <label key={i} className={`batch-row ${selected.has(i) ? "" : "off"}`}>
              <input type="checkbox" checked={selected.has(i)} onChange={() => toggle(i)} />
              <FileIcon name={it.name} size={22} />
              <span className="batch-name" title={it.dir ? `${it.dir}/${it.name}` : it.name}>
                {it.dir && <span className="batch-dir">{it.dir}/</span>}
                {it.name}
              </span>
              <span className="batch-size">{it.size != null ? bytes(it.size) : "--"}</span>
            </label>
          ))}
        </div>
      </div>

      <div className="field">
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
        <label className="batch-folder small muted">
          <Switch checked={newFolder} onChange={setNewFolder} />
          新建文件夹「{safeSegment(batch.title)}」，保留子目录结构
        </label>
      </div>

      <div className="cap-foot">
        <span className={`small ${error || tooSmall ? "err" : "muted"}`}>
          {error ?? (tooSmall ? `磁盘空间不足（剩余 ${bytes(free)}）` : `已选 ${chosen.length} 个 · ${bytes(chosenSize)}`)}
        </span>
        <button className="btn glass" onClick={close}>
          取消
        </button>
        <button className="btn glass" disabled={busy || !chosen.length} onClick={() => submit(false)}>
          稍后下载
        </button>
        <button ref={startRef} className="btn primary" disabled={busy || !chosen.length} onClick={() => submit(true)}>
          开始下载
        </button>
      </div>
    </div>
  );
}
