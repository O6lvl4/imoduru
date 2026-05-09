use anyhow::Result;
use rusqlite::Connection;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

/// Element metadata stored for adaptive relocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementData {
    pub tag: String,
    pub text: String,
    pub attributes: HashMap<String, String>,
    pub path: String,
    pub parent_tag: Option<String>,
    pub parent_attribs: Option<HashMap<String, String>>,
    pub parent_text: Option<String>,
    pub siblings: Vec<String>,
}

/// Adaptive selector storage backed by SQLite.
pub struct AdaptiveStore {
    conn: Mutex<Connection>,
}

impl AdaptiveStore {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS storage (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                identifier TEXT NOT NULL,
                element_data TEXT NOT NULL,
                UNIQUE (url, identifier)
            )",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Save element data for a given URL and identifier.
    pub fn save(&self, url: &str, identifier: &str, data: &ElementData) -> Result<()> {
        let normalized_id = normalize_identifier(identifier);
        let json = serde_json::to_string(data)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO storage (url, identifier, element_data) VALUES (?1, ?2, ?3)",
            rusqlite::params![url, normalized_id, json],
        )?;
        Ok(())
    }

    /// Retrieve stored element data.
    pub fn retrieve(&self, url: &str, identifier: &str) -> Result<Option<ElementData>> {
        let normalized_id = normalize_identifier(identifier);
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT element_data FROM storage WHERE url = ?1 AND identifier = ?2")?;
        let result = stmt
            .query_row(rusqlite::params![url, normalized_id], |row| {
                row.get::<_, String>(0)
            })
            .ok();
        match result {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }
}

fn normalize_identifier(id: &str) -> String {
    let trimmed = id.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(trimmed.as_bytes());
    format!("{:x}", hasher.finalize())
}

// -- Element extraction from HTML

/// Extract ElementData from a scraper ElementRef.
pub fn element_to_data(el: ElementRef) -> ElementData {
    let tag = el.value().name().to_string();
    let text: String = el.text().collect::<Vec<_>>().join(" ");
    let text = text
        .chars()
        .take(500)
        .collect::<String>()
        .trim()
        .to_string();

    let mut attributes = HashMap::new();
    for (k, v) in el.value().attrs() {
        attributes.insert(k.to_string(), v.to_string());
    }

    let path = build_css_path(el);

    let (parent_tag, parent_attribs, parent_text) = if let Some(parent) = el.parent() {
        if let Some(parent_el) = ElementRef::wrap(parent) {
            let pt = parent_el.value().name().to_string();
            let mut pa = HashMap::new();
            for (k, v) in parent_el.value().attrs() {
                pa.insert(k.to_string(), v.to_string());
            }
            let ptxt: String = parent_el
                .text()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(200)
                .collect();
            (Some(pt), Some(pa), Some(ptxt.trim().to_string()))
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    let siblings: Vec<String> = el
        .parent()
        .map(|p| {
            p.children()
                .filter_map(ElementRef::wrap)
                .map(|s| s.value().name().to_string())
                .collect()
        })
        .unwrap_or_default();

    ElementData {
        tag,
        text,
        attributes,
        path,
        parent_tag,
        parent_attribs,
        parent_text,
        siblings,
    }
}

fn build_css_path(el: ElementRef) -> String {
    let mut parts = Vec::new();
    let mut current = Some(el);
    while let Some(e) = current {
        let mut s = e.value().name().to_string();
        if let Some(id) = e.value().attr("id") {
            s.push_str(&format!("#{id}"));
        } else if let Some(class) = e.value().attr("class") {
            let first = class.split_whitespace().next().unwrap_or("");
            if !first.is_empty() {
                s.push_str(&format!(".{first}"));
            }
        }
        parts.push(s);
        current = e.parent().and_then(ElementRef::wrap);
    }
    parts.reverse();
    parts.join(" > ")
}

// -- Similarity scoring (ported from Scrapling's SequenceMatcher approach)

/// Calculate similarity between stored ElementData and a candidate element.
/// Returns 0.0–100.0 percentage.
pub fn similarity_score(original: &ElementData, candidate: &ElementData) -> f64 {
    let mut score: f64 = 0.0;
    let mut checks: usize = 0;

    // 1. Tag match
    score += if original.tag == candidate.tag {
        1.0
    } else {
        0.0
    };
    checks += 1;

    // 2. Text similarity
    if !original.text.is_empty() {
        score += strsim::normalized_levenshtein(&original.text, &candidate.text);
        checks += 1;
    }

    // 3. Attributes similarity
    score += dict_similarity(&original.attributes, &candidate.attributes);
    checks += 1;

    // 4. Key attribute matching (class, id, href, src)
    for key in &["class", "id", "href", "src"] {
        if let Some(orig_val) = original.attributes.get(*key) {
            let cand_val = candidate
                .attributes
                .get(*key)
                .map(|s| s.as_str())
                .unwrap_or("");
            score += strsim::normalized_levenshtein(orig_val, cand_val);
            checks += 1;
        }
    }

    // 5. Path similarity
    score += strsim::normalized_levenshtein(&original.path, &candidate.path);
    checks += 1;

    // 6. Parent info
    if let Some(ref orig_parent) = original.parent_tag {
        if let Some(ref cand_parent) = candidate.parent_tag {
            score += strsim::normalized_levenshtein(orig_parent, cand_parent);
            checks += 1;

            if let (Some(ref op), Some(ref cp)) =
                (&original.parent_attribs, &candidate.parent_attribs)
            {
                score += dict_similarity(op, cp);
                checks += 1;
            }

            if let Some(ref opt) = original.parent_text {
                let cpt = candidate.parent_text.as_deref().unwrap_or("");
                if !opt.is_empty() {
                    score += strsim::normalized_levenshtein(opt, cpt);
                    checks += 1;
                }
            }
        }
    }

    // 7. Siblings
    if !original.siblings.is_empty() {
        let orig_sib = original.siblings.join(",");
        let cand_sib = candidate.siblings.join(",");
        score += strsim::normalized_levenshtein(&orig_sib, &cand_sib);
        checks += 1;
    }

    if checks == 0 {
        return 0.0;
    }
    ((score / checks as f64) * 100.0 * 100.0).round() / 100.0
}

fn dict_similarity(a: &HashMap<String, String>, b: &HashMap<String, String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let keys_a: Vec<&str> = a.keys().map(|k| k.as_str()).collect();
    let keys_b: Vec<&str> = b.keys().map(|k| k.as_str()).collect();
    let vals_a: Vec<&str> = a.values().map(|v| v.as_str()).collect();
    let vals_b: Vec<&str> = b.values().map(|v| v.as_str()).collect();

    let key_sim = strsim::normalized_levenshtein(&keys_a.join(","), &keys_b.join(","));
    let val_sim = strsim::normalized_levenshtein(&vals_a.join(","), &vals_b.join(","));
    key_sim * 0.5 + val_sim * 0.5
}

/// Relocate: find the best-matching element in HTML for a stored ElementData.
/// Returns (css_selector_used, score) pairs above the threshold.
pub fn relocate(html: &str, original: &ElementData, min_score: f64) -> Vec<(String, f64)> {
    let doc = Html::parse_document(html);
    // Search for elements with the same tag
    let tag_sel = Selector::parse(&original.tag).unwrap_or_else(|_| Selector::parse("*").unwrap());

    let mut candidates: Vec<(String, f64)> = Vec::new();

    for el in doc.select(&tag_sel) {
        let cand_data = element_to_data(el);
        let score = similarity_score(original, &cand_data);
        if score >= min_score {
            let css = build_css_path(el);
            candidates.push((css, score));
        }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_retrieve() {
        let store = AdaptiveStore::open(":memory:").unwrap();
        let data = ElementData {
            tag: "div".into(),
            text: "hello world".into(),
            attributes: [("class".into(), "main".into())].into(),
            path: "html > body > div.main".into(),
            parent_tag: Some("body".into()),
            parent_attribs: None,
            parent_text: None,
            siblings: vec!["div".into(), "p".into()],
        };
        store.save("https://example.com", "test-el", &data).unwrap();
        let retrieved = store
            .retrieve("https://example.com", "test-el")
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.tag, "div");
        assert_eq!(retrieved.text, "hello world");
    }

    #[test]
    fn test_similarity_identical() {
        let data = ElementData {
            tag: "p".into(),
            text: "some text".into(),
            attributes: HashMap::new(),
            path: "html > body > p".into(),
            parent_tag: None,
            parent_attribs: None,
            parent_text: None,
            siblings: vec![],
        };
        let score = similarity_score(&data, &data);
        assert!((score - 100.0).abs() < 0.01);
    }
}
