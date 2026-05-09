//! HTML-to-readable-text extraction shared across web tools.
//!
//! # What it does
//!
//! `html_to_readable_text` walks a parsed HTML document and collects visible
//! text nodes while skipping non-content elements (`<script>`, `<style>`,
//! `<nav>`, `<footer>`, `<header>`, `<aside>`, `<noscript>`, `<svg>`,
//! `<iframe>`). Block-level elements (`<p>`, `<div>`, `<h1>`–`<h6>`, `<li>`,
//! etc.) are followed by a newline to preserve paragraph structure.
//!
//! `extract_title` pulls the first `<title>` tag from the document head.
//! Both functions are pure and operate only on already-downloaded HTML strings,
//! so they have no network or security surface of their own.

use scraper::{Html, Selector};

pub(crate) fn html_to_readable_text(html: &str, max_chars: usize) -> String {
    let document = Html::parse_document(html);

    let skip_selectors = [
        "script", "style", "nav", "footer", "header", "aside", "noscript", "svg", "iframe",
    ];
    let skip: Vec<Selector> = skip_selectors
        .iter()
        .filter_map(|s| Selector::parse(s).ok())
        .collect();

    let body_selector = Selector::parse("body").ok();
    let root = body_selector
        .as_ref()
        .and_then(|sel| document.select(sel).next());

    let text_source = root.unwrap_or_else(|| document.root_element());

    let mut text = String::with_capacity(max_chars);
    collect_text_recursive(&text_source, &skip, &mut text, max_chars);

    normalize_whitespace(&text, max_chars)
}

fn collect_text_recursive(
    element: &scraper::ElementRef<'_>,
    skip: &[Selector],
    buf: &mut String,
    max: usize,
) {
    if buf.len() >= max {
        return;
    }

    for child in element.children() {
        if buf.len() >= max {
            return;
        }
        match child.value() {
            scraper::node::Node::Text(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if !buf.is_empty() && !buf.ends_with('\n') && !buf.ends_with(' ') {
                        buf.push(' ');
                    }
                    buf.push_str(trimmed);
                }
            }
            scraper::node::Node::Element(_) => {
                if let Some(child_ref) = scraper::ElementRef::wrap(child) {
                    let should_skip = skip.iter().any(|sel| sel.matches(&child_ref));
                    if !should_skip {
                        let tag = child_ref.value().name();
                        let block_tag = matches!(
                            tag,
                            "p" | "div"
                                | "h1"
                                | "h2"
                                | "h3"
                                | "h4"
                                | "h5"
                                | "h6"
                                | "li"
                                | "br"
                                | "tr"
                                | "blockquote"
                                | "article"
                                | "section"
                        );
                        if block_tag && !buf.is_empty() && !buf.ends_with('\n') {
                            buf.push('\n');
                        }
                        collect_text_recursive(&child_ref, skip, buf, max);
                        if block_tag && !buf.ends_with('\n') {
                            buf.push('\n');
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn extract_title(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
fn extract_meta_description(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse(r#"meta[name="description"]"#).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn normalize_whitespace(text: &str, max_chars: usize) -> String {
    let mut result = String::with_capacity(text.len().min(max_chars));
    let mut prev_blank = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !prev_blank && !result.is_empty() {
                result.push('\n');
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;
        result.push_str(trimmed);
        result.push('\n');

        if result.len() >= max_chars {
            result.truncate(max_chars);
            break;
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_simple_html() {
        let html = "<html><body><p>Hello world</p></body></html>";
        let text = html_to_readable_text(html, 1000);
        assert!(text.contains("Hello world"));
    }

    #[test]
    fn strips_script_and_style() {
        let html = "<html><body>\
            <script>alert('xss')</script>\
            <style>.foo{}</style>\
            <p>Visible text</p>\
            </body></html>";
        let text = html_to_readable_text(html, 1000);
        assert!(text.contains("Visible text"));
        assert!(!text.contains("alert"));
        assert!(!text.contains(".foo"));
    }

    #[test]
    fn respects_max_chars() {
        let html = "<html><body><p>A very long paragraph that goes on and on</p></body></html>";
        let text = html_to_readable_text(html, 10);
        assert!(text.len() <= 15);
    }

    #[test]
    fn extracts_title_tag() {
        let html = "<html><head><title>My Page</title></head><body></body></html>";
        assert_eq!(extract_title(html), Some("My Page".to_string()));
    }

    #[test]
    fn extracts_meta_description() {
        let html = r#"<html><head><meta name="description" content="A great page"></head></html>"#;
        assert_eq!(
            extract_meta_description(html),
            Some("A great page".to_string())
        );
    }
}
