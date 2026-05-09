use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

/// Extract all links from HTML that share the same origin and match the path prefix.
pub fn extract_links(html: &str, base: &Url, path_prefix: &str) -> Vec<Url> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href]").unwrap();

    let mut links = Vec::new();
    for el in doc.select(&sel) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };

        let Ok(resolved) = base.join(href) else {
            continue;
        };

        if resolved.origin() != base.origin() {
            continue;
        }

        if !resolved.path().starts_with(path_prefix) {
            continue;
        }

        let mut clean = resolved.clone();
        clean.set_fragment(None);

        links.push(clean);
    }

    links.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    links.dedup();
    links
}

/// Extract PDF links from HTML (same origin, any path).
pub fn extract_pdf_links(html: &str, base: &Url) -> Vec<Url> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href]").unwrap();

    let mut links = Vec::new();
    for el in doc.select(&sel) {
        let Some(href) = el.value().attr("href") else {
            continue;
        };

        let Ok(resolved) = base.join(href) else {
            continue;
        };

        if resolved.path().to_lowercase().ends_with(".pdf") {
            links.push(resolved);
        }
    }

    links.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    links.dedup();
    links
}

const SKIP_TAGS: &[&str] = &[
    "script", "style", "nav", "header", "footer", "noscript", "iframe",
];

const BLOCK_TAGS: &[&str] = &[
    "p", "div", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "br", "dt", "dd", "th", "td",
    "section", "blockquote",
];

/// Extract readable text content from HTML, stripping nav/header/footer/script.
pub fn extract_text(html: &str) -> String {
    let doc = Html::parse_document(html);

    let main_sel = Selector::parse("main, article, .content, #content, #main, .main").ok();
    let body_sel = Selector::parse("body").unwrap();

    let root = main_sel
        .as_ref()
        .and_then(|s| doc.select(s).next())
        .or_else(|| doc.select(&body_sel).next());

    let Some(root) = root else {
        return String::new();
    };

    let mut text = String::new();
    collect_text_from_element(root, &mut text);
    text
}

fn collect_text_from_element(el: ElementRef, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Text(t) => {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push(' ');
                    }
                    out.push_str(trimmed);
                }
            }
            Node::Element(el_data) => {
                let tag = el_data.name();
                if SKIP_TAGS.contains(&tag) {
                    continue;
                }
                let is_block = BLOCK_TAGS.contains(&tag);
                if is_block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                if let Some(child_el) = ElementRef::wrap(child) {
                    collect_text_from_element(child_el, out);
                }
                if is_block && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
}

/// Extract page title.
pub fn extract_title(html: &str) -> Option<String> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("title").unwrap();
    doc.select(&sel)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
}
