import { FolderOpen, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { animateWindowHeight, api, pickDirectory, type CapturePayload, type Task } from "../lib/api";
import { bytes } from "../lib/format";
import { CaptureProgress } from "./CaptureProgress";
import { FileIcon } from "./TaskCard";
import { CaptureControls } from "./CaptureControls";

/** 展开为进度窗口后的高度 */
const PROGRESS_H = 318;

function nameFromUrl(url: string) {
  try {
    const seg = new URL(url).pathname.split("/").filter(Boolean).pop();
    return seg ? decodeURIComponent(seg) : "";
  } catch {
    return "";
  }
}

function hostOf(url: string | null | undefined) {
  try {
    return url ? new URL(url).hostname : "";
  } catch {
    return "";
  }
}

/** 下载确认小卡片：浏览器扩展转交下载时弹出的独立窗口 */
export function CaptureCard({ id }: { id: string }) {
  const [cap, setCap] = useState<CapturePayload | null>(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [dir, setDir] = useState("");
  const [free, setFree] = useState<number | null>(null);
  const [duplicate, setDuplicate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 开始下载后展开为进度视图 */
  const [task, setTask] = useState<Task | null>(null);
  const startRef = useRef<HTMLButtonElement>(null);

  const close = () => api.closeCapture(id);

  useEffect(() => {
    Promise.all([api.getCapture(id), api.getSettings(), api.getTasks()]).then(([c, s, tasks]) => {
      if (!c) return close();
      document.documentElement.dataset.glass = s.glass;
      setCap(c);
      setName(c.fileName || nameFromUrl(c.url));
      setDir(s.downloadDir);
      setDuplicate(tasks.some((t) => t.url === c.url));
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  // 内容渲染完成后再显示窗口，并聚焦「开始下载」（回车即可确认）
  useEffect(() => {
    if (!cap) return;
    api.captureReady().finally(() => startRef.current?.focus());
  }, [cap]);

  useEffect(() => {
    if (!dir) return;
    api.diskInfo(dir).then((d) => setFree(d?.free ?? null)).catch(() => setFree(null));
  }, [dir]);

  const submit = async (start: boolean) => {
    if (!cap || busy) return;
    setBusy(true);
    setError(null);
    try {
      const [created] = await api.addTasks([cap.url], {
        saveDir: dir,
        // 未改名且浏览器没给出文件名时交给后端按服务器响应识别
        fileName: nameEdited || cap.fileName ? name : undefined,
        start,
        headers: cap.headers,
        hls: cap.hls,
      });
      if (start && created) {
        // 原地展开成进度窗口
        setTask(created);
        animateWindowHeight(PROGRESS_H);
      } else {
        close();
      }
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (task) return; // 进度视图不响应确认快捷键
      if (e.key === "Escape") close();
      else if (e.key === "Enter" && !(e.target instanceof HTMLButtonElement)) submit(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  if (!cap) return <div className="capture" />;
  if (task) {
    return (
      <div className="capture expanded">
        <CaptureProgress initial={task} onClose={close} />
      </div>
    );
  }
  const tooSmall = free != null && cap.fileSize != null && free < cap.fileSize;

  return (
    <div className="capture" data-tauri-drag-region>
      <div className="cap-head" data-tauri-drag-region>
        <FileIcon name={name || cap.url} size={46} />
        <div className="cap-title" data-tauri-drag-region>
          <input
            className="cap-name"
            value={name}
            spellCheck={false}
            placeholder="自动识别文件名"
            onChange={(e) => {
              setName(e.target.value);
              setNameEdited(true);
            }}
          />
          <div className="cap-meta" data-tauri-drag-region>
            <span>{cap.hls || /\.m3u8(?:\?|$)/i.test(cap.url) ? "HLS · 自动清晰度 · 暂停后重下" : cap.fileSize ? bytes(cap.fileSize) : "大小未知"}</span>
            <span>{hostOf(cap.referrer) || hostOf(cap.url)}</span>
          </div>
        </div>
        <CaptureControls />
        <button className="icon-btn ghost" onClick={close} aria-label="取消">
          <X size={17} />
        </button>
      </div>

      <div className="cap-url mono" title={cap.url}>
        {cap.url}
      </div>

      <div className="field">
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

      <div className="cap-foot">
        <span className={`small ${error || tooSmall ? "err" : duplicate ? "warn" : "muted"}`}>
          {error ?? (tooSmall ? "磁盘空间不足" : duplicate ? "该链接已在任务列表中" : free != null ? `磁盘剩余 ${bytes(free)}` : "")}
        </span>
        <button className="btn glass" onClick={close}>
          取消
        </button>
        <button className="btn glass" disabled={busy} onClick={() => submit(false)}>
          稍后下载
        </button>
        <button ref={startRef} className="btn primary" disabled={busy} onClick={() => submit(true)}>
          开始下载
        </button>
      </div>
    </div>
  );
}
