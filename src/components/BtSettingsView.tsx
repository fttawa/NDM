import { useEffect, useState } from "react";
import { Network, RotateCcw, Save } from "lucide-react";
import { api, type BtSettings } from "../lib/api";
import { useStore } from "../store";
import { Switch } from "./ui";

export function SettingsTabs({ active }: { active: "general" | "bt" }) {
  const setView = useStore(s => s.setView);
  return <nav className="settings-tabs" aria-label="设置分类">
    <button className={`chip-btn ${active === "general" ? "active" : ""}`} aria-current={active === "general" ? "page" : undefined} onClick={() => setView("settings")}>通用设置</button>
    <button className={`chip-btn ${active === "bt" ? "active" : ""}`} aria-current={active === "bt" ? "page" : undefined} onClick={() => setView("bt-settings")}>BT 下载</button>
  </nav>;
}

const lines = (value: string) => value.split(/\r?\n/).map(line => line.trim()).filter(Boolean);

export function BtSettingsView() {
  const saved = useStore(s => s.settings?.bt);
  const [draft, setDraft] = useState<BtSettings | null>(null);
  const [nodes, setNodes] = useState("");
  const [trackers, setTrackers] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const showToast = useStore(s => s.showToast);
  const load = (bt: BtSettings) => { setDraft(structuredClone(bt)); setNodes(bt.bootstrapNodes.join("\n")); setTrackers(bt.trackers.join("\n")); setError(""); };
  useEffect(() => { if (saved) load(saved); }, [saved]);
  if (!draft || !saved) return null;
  const change = <K extends keyof BtSettings>(key: K, value: BtSettings[K]) => setDraft({ ...draft, [key]: value });
  const next = { ...draft, bootstrapNodes: lines(nodes), trackers: lines(trackers) };
  const dirty = JSON.stringify(next) !== JSON.stringify(saved);
  return <div className="view bt-settings">
    <header className="view-head"><div><h1>BT 下载配置</h1><p className="muted">管理资源发现与连接，应用后对正在下载的 BT 任务生效</p></div></header>
    <SettingsTabs active="bt" />
    <form onSubmit={async e => {
      e.preventDefault(); const settings = useStore.getState().settings; if (!settings || busy) return;
      setBusy(true); setError("");
      try { const updated = await api.setSettings({ ...settings, bt: next }); useStore.setState({ settings: updated }); showToast("BT 配置已保存并应用"); }
      catch (e) { setError(String(e)); }
      finally { setBusy(false); }
    }}>
      <fieldset disabled={busy} className="bt-fieldset">
        <section className="bt-settings-grid">
          <div className="tile glass set-tile">
            <div className="set-head"><div className="set-icon"><Network size={18} /></div><div><div className="set-title">资源发现</div><div className="muted small">DHT 与 PEX 默认开启</div></div></div>
            <div className="set-body">
              {([
                ["dht", "DHT 网络", "通过分布式网络寻找 Peer"],
                ["pex", "Peer 交换（PEX）", "从已连接的 Peer 获取更多节点"],
                ["lsd", "局域网发现（LSD）", "寻找同一局域网中的 Peer"],
                ["upnp", "自动端口映射", "使用 UPnP / NAT-PMP 尝试建立入站连接"],
              ] as const).map(([key, title, desc]) => <div className="set-row" key={key}><div><div id={`bt-${key}`}>{title}</div><div className="muted small">{desc}</div></div><Switch label={title} checked={draft[key]} onChange={v => change(key, v)} /></div>)}
            </div>
          </div>
          <div className="tile glass set-tile">
            <div className="set-title">连接与上传</div>
            <label className="field"><span>每任务 Peer 上限</span><input className="input" type="number" min={10} max={500} required value={draft.maxPeers} onChange={e => change("maxPeers", Number(e.target.value))} /><span className="muted small">10–500，默认 80；独立于 HTTP 连接数</span></label>
            <label className="field"><span>每任务上传限速（KB/s）</span><input className="input" type="number" min={0} max={1000000} required value={draft.uploadLimit} onChange={e => change("uploadLimit", Number(e.target.value))} /><span className="muted small">0 为不限速；下载完成后停止做种</span></label>
            <p className="muted small">下载限速与并发队列沿用通用设置。BT 连接目前直接联网。</p>
          </div>
          <div className="tile glass set-tile">
            <label className="field" htmlFor="bt-nodes"><b>DHT 引导节点</b><span className="muted small">每行一个 host:port，内置 3 个引导节点，可自行修改</span></label>
            <textarea id="bt-nodes" className="input mono bt-addresses" spellCheck={false} value={nodes} onChange={e => setNodes(e.target.value)} rows={5} />
            <p className="muted small">引导节点帮助加入 DHT 网络，并非资源服务器。启用 DHT 时至少保留一项；网络不可达时可换用其他节点。</p>
          </div>
          <div className="tile glass set-tile">
            <label className="field" htmlFor="bt-trackers"><b>补充 Tracker</b><span className="muted small">每行一个 UDP / HTTP / HTTPS 地址，最多 128 条</span></label>
            <textarea id="bt-trackers" className="input mono bt-addresses" spellCheck={false} value={trackers} onChange={e => setTrackers(e.target.value)} rows={5} />
            <p className="muted small">内置 3 个公共 Tracker，与种子自带的地址合并去重。清空可停止补充；私有种子只使用原有 Tracker，并禁用 DHT / PEX。</p>
          </div>
        </section>
        {error && <p className="err" role="alert">{error}</p>}
        <div className="bt-settings-actions">
          <button className="btn glass" type="button" onClick={async () => { setBusy(true); try { load(await api.btDefaults()); } catch (e) { setError(String(e)); } finally { setBusy(false); } }}><RotateCcw size={15} /> 恢复默认</button>
          <span className="muted small">{dirty ? "有未应用的修改" : "当前配置已保存"}</span>
          <button className="btn primary" type="submit" disabled={!dirty}><Save size={15} /> {busy ? "应用中…" : "应用配置"}</button>
        </div>
      </fieldset>
    </form>
  </div>;
}
