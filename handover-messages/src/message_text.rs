//! Safe rendering helpers for message bodies.

const MAX_LINKS: usize = 32;

/// Build a selectable label with escaped message text, safe web links, and an
/// optional search-term highlighting.
pub fn label(text: &str, query: Option<&str>) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_use_markup(true);
    label.set_selectable(true);
    label.set_markup(&markup(text, query));
    label
}

/// Return the visible HTTP(S) URLs in a message, for copy-link actions.
pub fn links(text: &str) -> Vec<String> {
    find_links(text)
        .into_iter()
        .map(|(start, end)| text[start..end].to_owned())
        .collect()
}

fn markup(text: &str, query: Option<&str>) -> String {
    let link_ranges = find_links(text);
    let highlights = query
        .filter(|query| !query.is_empty())
        .map(|query| {
            let (folded, positions) = casefold_with_map(text);
            let mut ranges = query
                .split_whitespace()
                .take(16)
                .flat_map(|term| find_in_folded(text, &folded, &positions, term))
                .collect::<Vec<_>>();
            ranges.sort_unstable();
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (start, end) in ranges {
                if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
                    last.1 = last.1.max(end);
                } else {
                    merged.push((start, end));
                }
            }
            merged
        })
        .unwrap_or_default();

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in link_ranges {
        append_plain(&mut out, text, cursor, start, &highlights);
        let href = gtk::glib::markup_escape_text(&text[start..end]);
        out.push_str("<a href=\"");
        out.push_str(href.as_str());
        out.push_str("\">");
        append_plain(&mut out, text, start, end, &highlights);
        out.push_str("</a>");
        cursor = end;
    }
    append_plain(&mut out, text, cursor, text.len(), &highlights);
    out
}

fn append_plain(
    out: &mut String,
    text: &str,
    start: usize,
    end: usize,
    highlights: &[(usize, usize)],
) {
    let mut cursor = start;
    for &(hl_start, hl_end) in highlights {
        let hs = hl_start.max(start);
        let he = hl_end.min(end);
        if hs >= he || hs < cursor {
            continue;
        }
        push_escaped(out, &text[cursor..hs]);
        out.push_str("<span background=\"#f6d365\" foreground=\"#202020\">");
        push_escaped(out, &text[hs..he]);
        out.push_str("</span>");
        cursor = he;
    }
    push_escaped(out, &text[cursor..end]);
}

fn push_escaped(out: &mut String, text: &str) {
    out.push_str(gtk::glib::markup_escape_text(text).as_str());
}

fn find_links(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut links = Vec::new();
    let mut i = 0;
    while i < bytes.len() && links.len() < MAX_LINKS {
        let scheme_len = if text[i..]
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
        {
            7
        } else if text[i..]
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        {
            8
        } else {
            i += text[i..].chars().next().unwrap().len_utf8();
            continue;
        };
        // Avoid finding a URL embedded in a word or another scheme.
        if i > 0
            && text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            i += 1;
            continue;
        }
        let mut end = i + scheme_len;
        while end < bytes.len() {
            let ch = text[end..].chars().next().unwrap();
            if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'' | '`') {
                break;
            }
            end += ch.len_utf8();
        }
        end = trim_url_end(text, i, end);
        if end > i + scheme_len {
            links.push((i, end));
            i = end;
        } else {
            i += scheme_len;
        }
    }
    links
}

fn trim_url_end(text: &str, start: usize, mut end: usize) -> usize {
    while let Some(ch) = text[start..end].chars().next_back() {
        if matches!(ch, '.' | ',' | '!' | '?' | ':' | ';') {
            end -= ch.len_utf8();
            continue;
        }
        let (open, close) = match ch {
            ')' => ('(', ')'),
            ']' => ('[', ']'),
            '}' => ('{', '}'),
            _ => break,
        };
        let candidate = &text[start..end];
        if candidate.matches(close).count() > candidate.matches(open).count() {
            end -= ch.len_utf8();
            continue;
        }
        break;
    }
    end
}

#[cfg(test)]
fn find_highlights(text: &str, query: &str) -> Vec<(usize, usize)> {
    let (haystack, map) = casefold_with_map(text);
    find_in_folded(text, &haystack, &map, query)
}
fn find_in_folded(
    text: &str,
    haystack: &str,
    map: &[(usize, usize, bool)],
    query: &str,
) -> Vec<(usize, usize)> {
    let needle = gtk::glib::casefold(query);
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(needle.as_str()) {
        let start = from + offset;
        let end = start + needle.len();
        // Only accept matches aligned to original Unicode character boundaries.
        let aligned_start = start == 0 || map[start - 1].0 != map[start].0;
        if start < map.len() && end > 0 && aligned_start && map[end - 1].2 {
            let original_start = map[start].0;
            let original_end = map[end - 1].1;
            if text.is_char_boundary(original_start) && text.is_char_boundary(original_end) {
                out.push((original_start, original_end));
            }
        }
        from = start + needle.len().max(1);
        if from >= haystack.len() {
            break;
        }
    }
    out
}

// Each lowercased byte maps to its source character span. The final flag marks
// the last byte produced by that source character, so matches cannot end mid-char.
fn casefold_with_map(text: &str) -> (String, Vec<(usize, usize, bool)>) {
    let mut lowered = String::new();
    let mut map = Vec::new();
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        if ch.is_ascii() {
            lowered.push(ch.to_ascii_lowercase());
            map.push((start, end, true));
            continue;
        }
        let mut buffer = [0; 4];
        let lower = gtk::glib::casefold(ch.encode_utf8(&mut buffer));
        let byte_count = lower.len();
        lowered.push_str(&lower);
        for i in 0..byte_count {
            map.push((start, end, i + 1 == byte_count));
        }
    }
    (lowered, map)
}

#[cfg(test)]
mod tests {
    use super::{find_highlights, find_links, markup};

    #[test]
    fn escapes_markup_and_url_attributes() {
        let rendered = markup("<b>x</b> https://example.test/?a=1&b=2", None);
        assert!(!rendered.contains("<b>"));
        assert!(rendered.contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(rendered.contains("href=\"https://example.test/?a=1&amp;b=2\""));
    }

    #[test]
    fn parses_urls_and_trims_sentence_punctuation_but_keeps_balanced_parens() {
        let text = "See https://example.test/a_(b), then http://example.test/x?!";
        let found = find_links(text);
        assert_eq!(
            found.iter().map(|&(s, e)| &text[s..e]).collect::<Vec<_>>(),
            ["https://example.test/a_(b)", "http://example.test/x"]
        );
    }

    #[test]
    fn permits_only_explicit_http_schemes_and_caps_results() {
        assert_eq!(
            find_links("www.example.test file:///tmp data:text/html javascript:alert(1)").len(),
            0
        );
        let text = (0..40)
            .map(|i| format!("https://e.test/{i} "))
            .collect::<String>();
        assert_eq!(find_links(&text).len(), 32);
    }

    #[test]
    fn highlights_separate_query_terms_and_merges_overlaps() {
        let rendered = markup("red apple and fresh red", Some("RED fresh re"));
        assert_eq!(rendered.matches("<span ").count(), 3);
    }

    #[test]
    fn unicode_case_insensitive_highlighting_maps_original_bytes() {
        let text = "Straße Σίσυφος";
        assert_eq!(find_highlights(text, "STRASSE"), [(0, "Straße".len())]);
        assert_eq!(
            find_highlights(text, "σίσυφος"),
            [("Straße ".len(), text.len())]
        );
        let rendered = markup(text, Some("STRASSE"));
        assert!(rendered.contains(">Straße</span>"));
    }
}
