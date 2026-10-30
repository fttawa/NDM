import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { api, type UpdateInfo } from "../lib/api";
import { bytes } from "../lib/format";
import { useLatest, usePresence } from "./Animated";

export const updateLabels: Record<UpdateInfo["phase"], string> = {
  idle: "尚未检查", checking: "正在检查更新", current: "当前已是最新版本", available: "新版本待更新",
  downloading: "正在下载更新", verifying: "正在校验签名", saving: "正在保存任务进度", installing: "正在安装，即将自动重启", error: "更新未完成",
};
export const updateBusy = (status: UpdateInfo | null) => !!status && ["checking", "downloading", "verifying", "saving", "installing"].includes(status.phase);

export function useUpdateStatus() {
  const [status, setStatus] = useState<UpdateInfo | null>(null);
  useEffect(() => {
    let disposed = false;
    const off = api.onUpdate(value => { if (!disposed) setStatus(value); });
    void api.updateStatus().then(value => { if (!disposed) setStatus(value); }).catch(() => {});
    return () => { disposed = true; void off.then(fn => fn()); };
  }, []);
  return [status, setStatus] as const;
}

export function UpdateNotice() {
  const [live] = useUpdateStatus();
  const show = !!live && ["downloading", "verifying", "saving", "installing"].includes(live.phase);
  const { mounted, closing } = usePresence(show, 220);
  const status = useLatest(live, show);
  if (!mounted || !status) return null;
  return <div className={`update-notice glass ${closing ? "closing" : ""}`} role="status"><RefreshCw size={16} /><div><b>{updateLabels[status.phase]}</b><p className="muted small">v{status.version}{status.phase === "downloading" ? ` · ${bytes(status.downloaded)} / ${bytes(status.total)}` : " · 安装后自动恢复原有下载任务"}</p></div></div>;
}
