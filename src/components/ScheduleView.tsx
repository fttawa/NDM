import { useEffect, useState } from "react";
import { Clock3, Power } from "lucide-react";
import { api, type CompletionStatus, type PowerAction, type Task } from "../lib/api";
import { dateTime } from "../lib/format";
import { useStore } from "../store";
import { SettingsTabs } from "./BtSettingsView";
import { Switch } from "./ui";
import { useLatest, usePresence } from "./Animated";

const time = (n: number) => `${String(Math.floor(n / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}`;
const minute = (v: string) => { const [h, m] = v.split(":").map(Number); return h * 60 + m; };
const actionName = (action: PowerAction | null) => action === "shutdown" ? "关机" : "睡眠";
export function useCompletion() {
  const [status, setStatus] = useState<CompletionStatus | null>(null);
  useEffect(() => {
    let disposed = false;
    const apply = (value: CompletionStatus) => { if (!disposed) setStatus(value); };
    const off = api.onCompletion(apply);
    void off.then(() => api.completionStatus()).then(apply).catch(() => {});
    return () => { disposed = true; void off.then(fn => fn()); };
  }, []);
  return status;
}

export function CompletionNotice() {
  const live = useCompletion(); const [now, setNow] = useState(Date.now());
  const toast = useStore(s => s.showToast);
  const show = !!(live?.deadline || live?.error);
  const { mounted, closing } = usePresence(show, 220);
  const status = useLatest(live, show);
  useEffect(() => { if (!status?.deadline) return; setNow(Date.now()); const timer = setInterval(() => setNow(Date.now()), 250); return () => clearInterval(timer); }, [status?.deadline]);
  if (!mounted || !status) return null;
  return <div className={`completion-notice glass-strong ${closing ? "closing" : ""}`} role="alert"><Power size={18} /><div><b>{status.error ? "电源操作未执行" : `${Math.max(0, Math.ceil((status.deadline! - now) / 1000))} 秒后${actionName(status.action)}`}</b><p className="muted small">{status.error || "全部任务已完成，可取消本次操作"}</p></div><button className="btn glass" onClick={() => void api.armCompletion(null).catch(e => toast(String(e)))}>{status.error ? "关闭" : "取消"}</button></div>;
}

export function ScheduleView() {
  const settings = useStore(s => s.settings); const tasks = useStore(s => s.tasks);
  const status = useCompletion(); const toast = useStore(s => s.showToast);
  const [enabled, setEnabled] = useState(false), [start, setStart] = useState("00:00"), [end, setEnd] = useState("08:00");
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const [action, setAction] = useState<PowerAction>("sleep");
  useEffect(() => { if (settings) { setEnabled(settings.downloadWindow.enabled); setStart(time(settings.downloadWindow.start)); setEnd(time(settings.downloadWindow.end)); } }, [settings?.downloadWindow]);
  const pending = tasks.filter(t => t.status !== "completed");
  const planned = tasks.filter(t => t.scheduledAt && t.status !== "completed");
  if (!settings) return null;
  return <div className="view"><header className="view-head"><div><h1>计划任务</h1><p className="muted">按系统本地时间运行，软件需保持打开或驻留托盘</p></div></header><SettingsTabs active="schedule" />
    <div className="schedule-grid">
      <form className="tile glass set-tile" onSubmit={async e => {
        e.preventDefault(); setBusy(true); setError("");
        try { const saved = await api.setSettings({ ...useStore.getState().settings!, downloadWindow: { enabled, start: minute(start), end: minute(end) } }); useStore.setState({ settings: saved }); toast("下载时段已应用"); }
        catch (e) { setError(String(e)); } finally { setBusy(false); }
      }}><div className="set-row"><div className="set-title"><Clock3 size={17} /> 允许下载时段</div><Switch label="限制下载时段" checked={enabled} disabled={busy} onChange={setEnabled} /></div>
        <div className="field-grid"><label className="field"><span>开始时间</span><input className="input" type="time" required disabled={busy} value={start} onChange={e => setStart(e.target.value)} /></label><label className="field"><span>结束时间</span><input className="input" type="time" required disabled={busy} value={end} onChange={e => setEnd(e.target.value)} /></label></div>
        <p className="muted small">每天重复，支持跨午夜。时段外停止 HTTP、HLS 与 BT 下载，时段内自动继续；手动暂停的任务保持暂停。HLS 恢复时从头下载。关闭限制即允许全天下载。</p>
        {error && <p className="err" role="alert">{error}</p>}<button className="btn primary" disabled={busy} type="submit">{busy ? "应用中…" : "应用下载时段"}</button>
      </form>
      <section className="tile glass set-tile"><div className="set-title"><Power size={17} /> 全部完成后</div><p className="muted small">本次运行有效，软件重启后关闭。仅全部任务成功完成且连接退出后触发；暂停、失败和等待中的任务都会阻止执行。开始 60 秒倒计时后可随时取消。</p>
        <label className="field"><span>完成后操作</span><select className="input" value={action} disabled={!!status?.action} onChange={e => setAction(e.target.value as PowerAction)}><option value="sleep">睡眠</option><option value="shutdown">关机</option></select></label>
        <div className="set-row"><span className="muted small">{status?.action ? `已启用：全部完成后${actionName(status.action)}` : `未启用 · ${pending.length} 个未完成任务`}</span><button className="btn glass" disabled={!status?.action && !pending.length} onClick={() => void api.armCompletion(status?.action ? null : action).catch(e => toast(String(e)))}>{status?.action ? "取消电源操作" : "启用本次操作"}</button></div>
        <p className="muted small">不会强制关闭其他程序。Windows 权限或未保存文档可能阻止电源操作。</p>
      </section>
    </div>
    <section className="tile glass schedule-tasks"><div className="set-title">定时开始的任务</div><p className="muted small">新建任务时选择「定时开始」，或暂停后在详情中设置。到期后进入队列，并遵守允许下载时段。</p>{planned.length ? planned.map(task => <button key={task.id} className="schedule-task" onClick={() => useStore.getState().select(task.id)}><span className="ellipsis">{task.fileName}</span><span>{dateTime(task.scheduledAt!)}</span><span className="muted small">{task.status === "paused" ? "手动暂停" : "等待开始"}</span></button>) : <p className="muted small">暂无定时任务</p>}</section>
  </div>;
}

export function TaskSchedule({ task }: { task: Task }) {
  const [value, setValue] = useState(""); const [busy, setBusy] = useState(false); const [error, setError] = useState("");
  useEffect(() => { const d = task.scheduledAt ? new Date(task.scheduledAt) : null; setValue(d ? `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,"0")}-${String(d.getDate()).padStart(2,"0")}T${time(d.getHours()*60+d.getMinutes())}` : ""); setError(""); }, [task.id, task.scheduledAt]);
  if (!["paused", "error", "waiting"].includes(task.status)) return null;
  const save = async (at: number | null) => { setBusy(true); setError(""); try { await api.setTaskSchedule(task.id, at); } catch(e) { setError(String(e)); } finally { setBusy(false); } };
  return <form className="task-schedule" onSubmit={e => { e.preventDefault(); const at = new Date(value).getTime(); if (!Number.isFinite(at) || at <= Date.now()) { setError("请选择未来的开始时间"); return; } void save(at); }}><label className="field"><span>定时开始</span><input className="input" type="datetime-local" required disabled={busy} value={value} onChange={e => setValue(e.target.value)} /></label><div className="toolbar-actions"><button className="btn glass" disabled={busy || !value}>保存计划</button>{task.scheduledAt && <button className="btn glass" type="button" disabled={busy} onClick={() => void save(null)}>取消定时</button>}</div>{task.scheduledAt && <p className="muted small">{task.status === "paused" ? "任务已手动暂停，需点击继续启用计划。" : "到期后进入队列，仍遵守允许下载时段。"}</p>}{error && <p className="err small" role="alert">{error}</p>}</form>;
}
