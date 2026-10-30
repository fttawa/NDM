import { Minus, Pin, PinOff } from "lucide-react";
import { useEffect, useState } from "react";
import { api, windowAction } from "../lib/api";

export function CaptureControls() {
  const [onTop, setOnTop] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let disposed = false;
    const apply = (s: { captureOnTop: boolean }) => { if (!disposed) setOnTop(s.captureOnTop); };
    const off = api.onSettings(apply);
    void off.then(() => api.getSettings()).then(apply).catch(e => { if (!disposed) setError(String(e)); });
    return () => { disposed = true; void off.then(fn => fn()); };
  }, []);
  return <div className="capture-controls">
    <button className="icon-btn ghost" disabled={busy} aria-label="下载小窗置顶" aria-pressed={onTop} title={error || (onTop ? "取消置顶（记住此选择）" : "置顶（记住此选择）")} onClick={async () => {
      setBusy(true);
      try { const settings = await api.getSettings(); const saved = await api.setSettings({ ...settings, captureOnTop: !settings.captureOnTop }); setOnTop(saved.captureOnTop); setError(""); }
      catch (e) { setError(String(e)); } finally { setBusy(false); }
    }}>{onTop ? <Pin size={15} /> : <PinOff size={15} />}</button>
    <button className="icon-btn ghost" aria-label="最小化下载小窗" title="最小化（继续在后台下载）" onClick={() => void windowAction("minimize").catch(e => setError(String(e)))}><Minus size={16} /></button>
  </div>;
}
