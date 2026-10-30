use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BtSettings {
    pub dht: bool,
    pub pex: bool,
    pub lsd: bool,
    pub upnp: bool,
    pub max_peers: u32,
    /// KiB/s, 0 means unlimited.
    pub upload_limit: u32,
    pub bootstrap_nodes: Vec<String>,
    pub trackers: Vec<String>,
    pub proxy: BtProxy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BtProxy {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl Default for BtProxy {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 1080,
            username: String::new(),
            password: String::new(),
        }
    }
}

impl BtProxy {
    fn validate(&mut self) -> Result<(), String> {
        self.host = self.host.trim().to_string();
        if !self.enabled {
            return Ok(());
        }
        let host = if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let url = reqwest::Url::parse(&format!("socks5://{host}:{}", self.port))
            .map_err(|_| "BT 代理主机或端口无效")?;
        if self.host.is_empty()
            || self.port == 0
            || self.host.contains(['/', '\\', '?', '#', '@'])
            || self.host.chars().any(char::is_whitespace)
            || url.host_str().is_none()
            || url.path() != "" && url.path() != "/"
        {
            return Err("BT 代理请填写主机名 / IP 与 1–65535 端口".into());
        }
        self.host = url.host_str().unwrap().trim_matches(['[', ']']).to_string();
        if self.username.len() > 255
            || self.password.len() > 255
            || self.username.contains('\0')
            || self.password.contains('\0')
            || (self.username.is_empty() && !self.password.is_empty())
        {
            return Err("SOCKS5 用户名和密码最多 255 字节；填写密码时也须填写用户名".into());
        }
        Ok(())
    }
}

impl Default for BtSettings {
    fn default() -> Self {
        Self {
            dht: true,
            pex: true,
            lsd: false,
            upnp: false,
            max_peers: 80,
            upload_limit: 0,
            bootstrap_nodes: vec![
                "dht.libtorrent.org:25401".into(),
                "router.bittorrent.com:6881".into(),
                "router.utorrent.com:6881".into(),
            ],
            trackers: vec![
                "udp://tracker.opentrackr.org:1337/announce".into(),
                "udp://open.stealth.si:80/announce".into(),
                "udp://tracker.torrent.eu.org:451/announce".into(),
            ],
            proxy: Default::default(),
        }
    }
}

impl BtSettings {
    pub fn validate(&mut self) -> Result<(), String> {
        self.proxy.validate()?;
        if !(10..=500).contains(&self.max_peers) {
            return Err("BT 每任务 Peer 上限须为 10–500".into());
        }
        if self.upload_limit > 1_000_000 {
            return Err("BT 上传限速须为 0–1000000 KB/s".into());
        }
        normalize(&mut self.bootstrap_nodes, 16, 260)?;
        normalize(&mut self.trackers, 128, 2048)?;
        if self.dht && self.bootstrap_nodes.is_empty() {
            return Err("启用 DHT 时请至少填写一个引导节点".into());
        }
        for node in &self.bootstrap_nodes {
            let url = reqwest::Url::parse(&format!("http://{node}"))
                .map_err(|_| "DHT 节点格式须为 host:port")?;
            if url.host_str().is_none()
                || url.port_or_known_default() == Some(0)
                || node
                    .rsplit_once(':')
                    .and_then(|(_, p)| p.parse::<u16>().ok())
                    .is_none_or(|p| p == 0)
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
                || node.contains('/')
            {
                return Err(format!(
                    "DHT 节点无效：{node}，请使用 host:port（IPv6 使用 [地址]:端口）"
                ));
            }
        }
        for tracker in &self.trackers {
            let url = reqwest::Url::parse(tracker).map_err(|_| "Tracker 地址无效")?;
            if !matches!(url.scheme(), "udp" | "http" | "https")
                || url.host_str().is_none()
                || url.fragment().is_some()
                || url.port() == Some(0)
                || !url.username().is_empty()
                || url.password().is_some()
                || (url.scheme() == "udp" && url.port().is_none())
            {
                return Err("Tracker 须为 udp://host:port/announce 或 HTTP(S) 地址".into());
            }
        }
        Ok(())
    }
}

fn normalize(values: &mut Vec<String>, max: usize, length: usize) -> Result<(), String> {
    if values.len() > max {
        return Err(format!("地址数量不能超过 {max} 条"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut normalized = Vec::new();
    for value in values.iter().map(|v| v.trim()).filter(|v| !v.is_empty()) {
        if value.len() > length || value.chars().any(char::is_whitespace) || value.contains(',') {
            return Err("地址过长或包含空白 / 逗号，请每行填写一个地址".into());
        }
        if seen.insert(value.to_string()) {
            normalized.push(value.to_string());
        }
    }
    *values = normalized;
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Runtime {
    pub phase: String,
    pub peers: u32,
    pub seeds: u32,
    pub upload_speed: u64,
    pub uploaded: u64,
    pub dht_nodes: u32,
    pub pieces: u32,
    pub total_pieces: u32,
    pub dht: bool,
    pub pex: bool,
    pub private: bool,
    pub max_peers: u32,
    pub upload_limit: u32,
    pub file_progress: Vec<u64>,
    pub trackers: Vec<Tracker>,
    pub proxy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracker {
    pub url: String,
    pub state: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socks5_proxy_validates_host_port_and_authentication() {
        for (host, normalized) in [
            ("  localhost ", "localhost"),
            ("127.0.0.1", "127.0.0.1"),
            ("::1", "::1"),
            ("[::1]", "::1"),
            ("proxy.example.com", "proxy.example.com"),
        ] {
            let mut proxy = BtProxy {
                enabled: true,
                host: host.into(),
                ..Default::default()
            };
            proxy.validate().unwrap();
            assert_eq!(proxy.host, normalized);
        }
        for host in [
            "",
            "http://localhost",
            "localhost/path",
            "user@host",
            "a b",
            "a?b",
            "a#b",
            "localhost:1080",
            "[::1",
            "a\\b",
        ] {
            let mut proxy = BtProxy {
                enabled: true,
                host: host.into(),
                ..Default::default()
            };
            assert!(proxy.validate().is_err(), "{host}");
        }
        let mut proxy = BtProxy {
            enabled: true,
            port: 0,
            ..Default::default()
        };
        assert!(proxy.validate().is_err());
        proxy.port = 1080;
        proxy.password = "secret".into();
        assert!(proxy.validate().is_err());
        proxy.username = "user".into();
        proxy.validate().unwrap();
        proxy.password = "密".repeat(86);
        assert!(proxy.validate().is_err());
        proxy.password = "bad\0password".into();
        assert!(proxy.validate().is_err());
        proxy.enabled = false;
        proxy.validate().unwrap();
    }
    #[test]
    fn old_settings_receive_defaults_and_lists_are_validated() {
        let old: crate::model::Settings = serde_json::from_str(r#"{"connections":8}"#).unwrap();
        assert_eq!(old.bt, BtSettings::default());
        let mut config = BtSettings::default();
        config.validate().unwrap();
        assert!(BtSettings {
            max_peers: 9,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(BtSettings {
            upload_limit: 1_000_001,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(BtSettings {
            bootstrap_nodes: vec![],
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(BtSettings {
            dht: false,
            bootstrap_nodes: vec![],
            trackers: vec![],
            ..Default::default()
        }
        .validate()
        .is_ok());
        assert!(BtSettings {
            trackers: vec!["http://localhost/announce".into(); 129],
            ..Default::default()
        }
        .validate()
        .is_err());
        config.trackers = vec![
            " udp://localhost:1337/announce ".into(),
            "udp://localhost:1337/announce".into(),
        ];
        config.bootstrap_nodes = vec!["[::1]:6881".into()];
        config.validate().unwrap();
        assert_eq!(config.trackers.len(), 1);
        for bad in [
            "file:///tmp/test",
            "ws://example.com/announce",
            "udp://example.com/announce",
            "https://a.invalid/one\ntwo",
        ] {
            let mut config = BtSettings {
                trackers: vec![bad.into()],
                ..Default::default()
            };
            assert!(config.validate().is_err(), "{bad}");
        }
        for bad in [
            "localhost",
            "a:0",
            "a:65536",
            "a:6881/path",
            "a:6881?x=1",
            "a:6881,b:6881",
        ] {
            let mut config = BtSettings {
                bootstrap_nodes: vec![bad.into()],
                ..Default::default()
            };
            assert!(config.validate().is_err(), "{bad}");
        }
    }
}
