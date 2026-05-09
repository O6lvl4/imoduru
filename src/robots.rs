use std::collections::HashMap;
use url::Url;

/// Minimal robots.txt parser.
/// Parses Allow/Disallow rules and Crawl-delay for a given user-agent.
pub struct RobotsManager {
    rules: HashMap<String, RobotsRules>,
    user_agent: String,
}

#[derive(Debug, Clone)]
struct RobotsRules {
    disallow: Vec<String>,
    allow: Vec<String>,
    crawl_delay: Option<f64>,
}

impl RobotsManager {
    pub fn new(user_agent: &str) -> Self {
        Self {
            rules: HashMap::new(),
            user_agent: user_agent.to_lowercase(),
        }
    }

    /// Fetch and parse robots.txt for a given origin.
    /// Uses the provided fetch function to get the content.
    pub fn load(&mut self, origin: &str, content: &str) {
        let rules = parse_robots_txt(content, &self.user_agent);
        self.rules.insert(origin.to_string(), rules);
    }

    /// Check if a URL is allowed by robots.txt.
    pub fn is_allowed(&self, url: &Url) -> bool {
        let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
        let Some(rules) = self.rules.get(&origin) else {
            return true; // No robots.txt loaded → allow
        };

        let path = url.path();

        // Check allow rules first (more specific wins per Google spec)
        for allow in &rules.allow {
            if path_matches(path, allow) {
                return true;
            }
        }

        for disallow in &rules.disallow {
            if disallow.is_empty() {
                continue;
            }
            if path_matches(path, disallow) {
                return false;
            }
        }

        true
    }

    /// Get the crawl delay for a given origin (seconds).
    pub fn crawl_delay(&self, origin: &str) -> Option<f64> {
        self.rules.get(origin).and_then(|r| r.crawl_delay)
    }
}

fn parse_robots_txt(content: &str, target_ua: &str) -> RobotsRules {
    let mut rules = RobotsRules {
        disallow: Vec::new(),
        allow: Vec::new(),
        crawl_delay: None,
    };

    let mut in_matching_group = false;
    let mut found_specific = false;

    // Collect rules for matching UA and wildcard
    let mut wildcard_rules = RobotsRules {
        disallow: Vec::new(),
        allow: Vec::new(),
        crawl_delay: None,
    };
    let mut in_wildcard_group = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim();

        match key.as_str() {
            "user-agent" => {
                let ua = value.to_lowercase();
                if ua == "*" {
                    in_wildcard_group = true;
                    in_matching_group = false;
                } else if target_ua.contains(&ua) || ua.contains(target_ua) {
                    in_matching_group = true;
                    in_wildcard_group = false;
                    found_specific = true;
                } else {
                    in_matching_group = false;
                    in_wildcard_group = false;
                }
            }
            "disallow" if in_matching_group => {
                rules.disallow.push(value.to_string());
            }
            "allow" if in_matching_group => {
                rules.allow.push(value.to_string());
            }
            "crawl-delay" if in_matching_group => {
                rules.crawl_delay = value.parse().ok();
            }
            "disallow" if in_wildcard_group => {
                wildcard_rules.disallow.push(value.to_string());
            }
            "allow" if in_wildcard_group => {
                wildcard_rules.allow.push(value.to_string());
            }
            "crawl-delay" if in_wildcard_group => {
                wildcard_rules.crawl_delay = value.parse().ok();
            }
            _ => {}
        }
    }

    // Use specific rules if found, otherwise fall back to wildcard
    if found_specific {
        rules
    } else {
        wildcard_rules
    }
}

fn path_matches(path: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        path.starts_with(prefix)
    } else if let Some(exact) = pattern.strip_suffix('$') {
        path == exact
    } else {
        path.starts_with(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_disallow() {
        let mut mgr = RobotsManager::new("imoduru");
        mgr.load(
            "https://example.com",
            "User-agent: *\nDisallow: /private/\nAllow: /private/public/\n",
        );
        let allowed = |path: &str| {
            mgr.is_allowed(&Url::parse(&format!("https://example.com{path}")).unwrap())
        };
        assert!(allowed("/"));
        assert!(!allowed("/private/secret"));
        assert!(allowed("/private/public/page"));
        assert!(allowed("/other"));
    }

    #[test]
    fn test_crawl_delay() {
        let mut mgr = RobotsManager::new("imoduru");
        mgr.load(
            "https://example.com",
            "User-agent: *\nCrawl-delay: 2.5\nDisallow:\n",
        );
        assert_eq!(mgr.crawl_delay("https://example.com"), Some(2.5));
    }
}
