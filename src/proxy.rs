use std::sync::Mutex;

/// Thread-safe proxy rotator with cyclic rotation strategy.
pub struct ProxyRotator {
    proxies: Vec<ProxyEntry>,
    index: Mutex<usize>,
}

#[derive(Debug, Clone)]
pub struct ProxyEntry {
    pub server: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl ProxyEntry {
    /// Parse proxy from string.
    /// Formats: "http://host:port", "http://user:pass@host:port", "socks5://host:port"
    pub fn parse(s: &str) -> Self {
        if let Ok(url) = url::Url::parse(s) {
            let username = if url.username().is_empty() {
                None
            } else {
                Some(url.username().to_string())
            };
            let password = url.password().map(|p| p.to_string());

            // Rebuild server URL without credentials
            let server = if let Some(host) = url.host_str() {
                let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
                format!("{}://{host}{port}", url.scheme())
            } else {
                s.to_string()
            };

            Self {
                server,
                username,
                password,
            }
        } else {
            Self {
                server: s.to_string(),
                username: None,
                password: None,
            }
        }
    }

    /// Serialize to JSON for the bridge.
    pub fn to_bridge_json(&self) -> serde_json::Value {
        let mut obj = serde_json::json!({ "server": self.server });
        if let Some(ref u) = self.username {
            obj["username"] = serde_json::json!(u);
        }
        if let Some(ref p) = self.password {
            obj["password"] = serde_json::json!(p);
        }
        obj
    }
}

impl ProxyRotator {
    /// Create from a list of proxy strings.
    pub fn new(proxies: Vec<String>) -> Self {
        let entries: Vec<ProxyEntry> = proxies.into_iter().map(|s| ProxyEntry::parse(&s)).collect();
        Self {
            proxies: entries,
            index: Mutex::new(0),
        }
    }

    /// Get next proxy (cyclic rotation).
    pub fn next(&self) -> Option<&ProxyEntry> {
        if self.proxies.is_empty() {
            return None;
        }
        let mut idx = self.index.lock().unwrap();
        let proxy = &self.proxies[*idx % self.proxies.len()];
        *idx = (*idx + 1) % self.proxies.len();
        Some(proxy)
    }

    pub fn len(&self) -> usize {
        self.proxies.len()
    }
}

/// Detect proxy-specific errors from error messages.
pub fn is_proxy_error(error_msg: &str) -> bool {
    let lower = error_msg.to_lowercase();
    let indicators = [
        "net::err_proxy",
        "net::err_tunnel",
        "connection refused",
        "connection reset",
        "connection timed out",
        "failed to connect",
        "could not resolve proxy",
        "proxy_connection_failed",
    ];
    indicators.iter().any(|ind| lower.contains(ind))
}

/// Load proxy list from a file (one proxy per line, # comments allowed).
pub fn load_proxy_file(path: &str) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let proxies: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    Ok(proxies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let p = ProxyEntry::parse("http://proxy.example.com:8080");
        assert_eq!(p.server, "http://proxy.example.com:8080");
        assert!(p.username.is_none());
    }

    #[test]
    fn test_parse_with_auth() {
        let p = ProxyEntry::parse("http://user:pass@proxy.example.com:8080");
        assert_eq!(p.server, "http://proxy.example.com:8080");
        assert_eq!(p.username.as_deref(), Some("user"));
        assert_eq!(p.password.as_deref(), Some("pass"));
    }

    #[test]
    fn test_rotation() {
        let rotator = ProxyRotator::new(vec![
            "http://a:1".into(),
            "http://b:2".into(),
            "http://c:3".into(),
        ]);
        assert_eq!(rotator.next().unwrap().server, "http://a:1");
        assert_eq!(rotator.next().unwrap().server, "http://b:2");
        assert_eq!(rotator.next().unwrap().server, "http://c:3");
        assert_eq!(rotator.next().unwrap().server, "http://a:1"); // wrap around
    }

    #[test]
    fn test_proxy_error_detection() {
        assert!(is_proxy_error("net::ERR_PROXY_CONNECTION_FAILED"));
        assert!(is_proxy_error("Connection refused"));
        assert!(!is_proxy_error("page not found"));
    }
}
