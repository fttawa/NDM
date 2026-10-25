import { Bell, Clipboard, Copy, FolderOpen, Gauge, Globe, Layers, Network, Palette, Plug, Power, Zap } from "lucide-react";

const GITHUB_URL = "https://github.com/fttawa/NDM";

/** GitHub 标志（Octicons mark-github；lucide 1.x 已移除品牌图标） */
function GithubMark({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0016 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}
import { useEffect, useState } from "react";
import { api, pickDirectory, pickExportPath, pickImportPath, writeClipboard, type BridgeStatus, type Settings } from "../lib/api";
import { useStore } from "../store";
import { Segmented, Slider, Switch } from "./ui";

function Tile({ icon, title, desc, className = "", children }: { icon: React.ReactNode; title: string; desc?: string; className?: string; children: React.ReactNode }) {
  return (
    <div className={`tile glass set-tile ${className}`}>
      <div className="set-head">
        <div className="set-icon">{icon}</div>
        <div>
          <div className="set-title">{title}</div>
          {desc && <div className="muted small">{desc}</div>}
        </div>
      </div>
      <div className="set-body">{children}</div>
    </div>
  );
}

export function SettingsView() {
  const settings = useStore((s) => s.settings);
  const save = useStore((s) => s.saveSettings);
  const [ua, setUa] = useState("");
  const [limit, setLimit] = useState("0");
  const [proxyUrl, setProxyUrl] = useState("");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null);
  // 选择「自定义」但尚未填写地址时，只在界面上切换，填写后再保存
  const [customPending, setCustomPending] = useState(false);
  const [bridge, setBridge] = useState<BridgeStatus | null>(null);
  const showToast = useStore((s) => s.showToast);

  // 定时刷新接入状态（端口被占用时后端会自动重试）
  useEffect(() => {
    const load = () => api.bridgeStatus().then(setBridge).catch(() => setBridge(null));
    load();
    const timer = setInterval(load, 2000);
    return () => clearInterval(timer);
  }, [settings?.browserIntegration]);

  const openExtensionDir = async (copy: boolean) => {
    try {
      const dir = await api.extensionDir();
      if (copy) {
        await writeClipboard(dir);
        showToast("扩展目录路径已复制");
      } else {
        await api.openDir(dir);
      }
    } catch (e) {
      showToast(String(e));
    }
  };

  useEffect(() => {
    if (settings) {
      setUa(settings.userAgent);
      setLimit(String(settings.speedLimit));
      setProxyUrl(settings.proxyUrl);
    }
  }, [settings]);

  if (!settings) return null;
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => save({ [k]: v } as Partial<Settings>);

  return (
    <div className="view">
      <header className="view-head">
        <div>
          <h1>设置</h1>
          <p className="muted">修改会自动保存</p>
        </div>
      </header>

      <section className="bento settings-bento">
        <Tile icon={<FolderOpen size={18} />} title="默认下载目录" desc="新任务默认保存的位置" className="span-2">
          <div className="input-group">
            <input className="input" value={settings.downloadDir} onChange={(e) => set("downloadDir", e.target.value)} />
            <button
              className="btn glass"
              onClick={async () => {
                const d = await pickDirectory(settings.downloadDir);
                if (d) set("downloadDir", d);
              }}
            >
              浏览
            </button>
          </div>
        </Tile>

        <Tile icon={<Palette size={18} />} title="外观" desc="窗口毛玻璃材质" className="span-2 row-2">
          <div className="set-row">
            <span>窗口材质</span>
            <Segmented
              value={settings.glass}
              onChange={(v) => set("glass", v)}
              options={[
                { value: "acrylic", label: "亚克力" },
                { value: "mica", label: "云母" },
                { value: "none", label: "关闭" },
              ]}
            />
          </div>
          <div className="glass-preview">
            <div className={`gp-card glass ${settings.glass}`}>
              <div className="gp-line" />
              <div className="gp-line short" />
            </div>
          </div>
          <p className="muted small">亚克力会透出并模糊桌面；云母更淡雅且省电（仅 Windows 11）。</p>
        </Tile>

        <Tile icon={<Layers size={18} />} title="同时下载任务数" desc="超出的任务将排队">
          <div className="big-value">{settings.maxConcurrent}</div>
          <Slider value={settings.maxConcurrent} min={1} max={16} onChange={(v) => set("maxConcurrent", v)} />
        </Tile>

        <Tile icon={<Zap size={18} />} title="每任务连接数" desc="分段并发数，越大越快">
          <div className="big-value">{settings.connections}</div>
          <Slider value={settings.connections} min={1} max={32} onChange={(v) => set("connections", v)} />
          <label className="set-row" title="测量请求到服务器开始传输数据的延迟，预判即将传完或重试中的连接，提前补上新连接防止掉速">
            <span>超频（最多 +{Math.floor(settings.connections / 2)}）</span>
            <Switch checked={settings.overclock} onChange={(v) => set("overclock", v)} />
          </label>
        </Tile>

        <Tile icon={<Gauge size={18} />} title="全局限速" desc="0 表示不限速" className="span-2">
          <div className="input-group">
            <input
              className="input"
              inputMode="numeric"
              value={limit}
              onChange={(e) => setLimit(e.target.value.replace(/\D/g, ""))}
              onBlur={() => set("speedLimit", Number(limit) || 0)}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            />
            <span className="input-suffix">KB/s</span>
          </div>
          <div className="chips">
            {[0, 512, 1024, 5120, 10240].map((v) => (
              <button key={v} className={`chip-btn ${settings.speedLimit === v ? "active" : ""}`} onClick={() => set("speedLimit", v)}>
                {v === 0 ? "不限" : v >= 1024 ? `${v / 1024} MB/s` : `${v} KB/s`}
              </button>
            ))}
          </div>
        </Tile>

        <Tile icon={<Network size={18} />} title="代理" desc="下载时使用的网络代理" className="span-2">
          <div className="set-row">
            <span>模式</span>
            <Segmented
              value={customPending ? "custom" : settings.proxyMode}
              onChange={(v) => {
                setTestResult(null);
                const pending = v === "custom" && !proxyUrl.trim();
                setCustomPending(pending);
                if (!pending) set("proxyMode", v);
              }}
              options={[
                { value: "system", label: "跟随系统" },
                { value: "none", label: "不使用" },
                { value: "custom", label: "自定义" },
              ]}
            />
          </div>
          {(customPending || settings.proxyMode === "custom") && (
            <input
              className="input mono"
              placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080"
              value={proxyUrl}
              onChange={(e) => setProxyUrl(e.target.value)}
              onBlur={async () => {
                if (!proxyUrl.trim()) return;
                if (customPending || proxyUrl.trim() !== settings.proxyUrl) {
                  await save({ proxyMode: "custom", proxyUrl });
                  // 保存失败（地址无效）时保持输入框可见，方便修改
                  if (useStore.getState().settings?.proxyMode === "custom") setCustomPending(false);
                }
              }}
              onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            />
          )}
          <div className="set-row">
            <span className={`small ${testResult ? (testResult.ok ? "ok" : "err") : "muted"}`}>
              {testResult ? testResult.msg : customPending || settings.proxyMode === "custom" ? "支持 http / https / socks5，可带用户名密码" : settings.proxyMode === "system" ? "使用 Windows 系统代理与环境变量" : settings.proxyMode === "none" ? "所有请求直连" : ""}
            </span>
            <button
              className="btn glass"
              disabled={testing}
              onClick={async () => {
                setTesting(true);
                setTestResult(null);
                try {
                  const ms = await api.testProxy({ ...settings, proxyMode: customPending ? "custom" : settings.proxyMode, proxyUrl });
                  setTestResult({ ok: true, msg: `连接正常，耗时 ${ms} ms` });
                } catch (e) {
                  setTestResult({ ok: false, msg: String(e) });
                } finally {
                  setTesting(false);
                }
              }}
            >
              {testing ? "测试中…" : "测试连接"}
            </button>
          </div>
        </Tile>

        <Tile icon={<Plug size={18} />} title="浏览器集成" desc="由扩展拦截 Chrome / Edge 的下载" className="span-2">
          <label className="set-row">
            <span>启用浏览器扩展接入</span>
            <Switch checked={settings.browserIntegration} onChange={(v) => set("browserIntegration", v)} />
          </label>
          <label className="set-row">
            <span>拦截到下载时弹出确认框</span>
            <Switch checked={settings.captureConfirm} onChange={(v) => set("captureConfirm", v)} />
          </label>
          <div className="set-row">
            <span className={`small ${bridge?.running ? "ok" : bridge?.error ? "warn" : "muted"}`}>
              {bridge?.running ? `正在监听 127.0.0.1:${bridge.port}` : bridge?.error ?? "已关闭"}
            </span>
            <div className="input-group">
              <button className="btn glass" onClick={() => openExtensionDir(true)} title="复制扩展目录路径">
                <Copy size={14} />
              </button>
              <button className="btn glass" onClick={() => openExtensionDir(false)}>
                <FolderOpen size={14} /> 扩展目录
              </button>
            </div>
          </div>
          <p className="muted small">
            安装：在浏览器打开 chrome://extensions（Edge 为 edge://extensions），开启「开发者模式」，点击「加载已解压的扩展程序」并选择扩展目录。
          </p>
        </Tile>

        <Tile icon={<Power size={18} />} title="行为" className="span-2">
          <div className="toggle-list">
            <label className="set-row">
              <span>
                <Power size={14} /> 关闭窗口时最小化到托盘
              </span>
              <Switch checked={settings.closeToTray} onChange={(v) => set("closeToTray", v)} />
            </label>
            <label className="set-row">
              <span>
                <Clipboard size={14} /> 监听剪贴板中的下载链接
              </span>
              <Switch checked={settings.clipboardWatch} onChange={(v) => set("clipboardWatch", v)} />
            </label>
            <label className="set-row">
              <span>
                <Bell size={14} /> 下载完成时通知
              </span>
              <Switch checked={settings.notify} onChange={(v) => set("notify", v)} />
            </label>
            <label className="set-row">
              <span>
                <Zap size={14} /> 启动时继续未完成的任务
              </span>
              <Switch checked={settings.resumeOnStart} onChange={(v) => set("resumeOnStart", v)} />
            </label>
          </div>
        </Tile>

        <Tile icon={<Globe size={18} />} title="User-Agent" desc="部分站点需要浏览器标识才能下载" className="span-2">
          <textarea className="input mono" rows={3} value={ua} onChange={(e) => setUa(e.target.value)} onBlur={() => ua !== settings.userAgent && set("userAgent", ua)} />
        </Tile>

        <Tile icon={<Copy size={18} />} title="数据备份" desc="导出或恢复任务与设置" className="span-2">
          <div className="set-row">
            <span className="muted small">导入前请先暂停所有下载任务</span>
            <div className="input-group">
              <button
                className="btn glass"
                onClick={async () => {
                  try {
                    const path = await pickExportPath("ndm-backup.json");
                    if (path) {
                      await api.exportData(path);
                      showToast("数据已导出");
                    }
                  } catch (e) {
                    showToast(String(e));
                  }
                }}
              >
                导出
              </button>
              <button
                className="btn glass"
                onClick={async () => {
                  try {
                    const path = await pickImportPath();
                    if (path) {
                      await api.importData(path);
                      await useStore.getState().reload();
                      showToast("数据已导入");
                    }
                  } catch (e) {
                    showToast(String(e));
                  }
                }}
              >
                导入
              </button>
            </div>
          </div>
        </Tile>

        <div className="tile glass span-2 about-tile">
          <div className="about-logo">
            <img src="/app-icon.svg" alt="" />
          </div>
          <div>
            <div className="set-title">NDM {__APP_VERSION__}</div>
            <div className="muted small">基于 Tauri 2 的多线程下载管理器 · 动态分段 · 断点续传</div>
            <button className="about-link" onClick={() => api.openUrl(GITHUB_URL)}>
              <GithubMark /> github.com/fttawa/NDM
            </button>
          </div>
        </div>
      </section>
    </div>
  );
}
