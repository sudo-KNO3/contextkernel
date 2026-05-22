//! KnowledgeItem → HTML `<section>` serialisation.
//!
//! Used when the server creates / approves a new item and needs to write it
//! back to a vault file. The on-disk `body_html` is stored verbatim so this
//! function is only the wrapping `<section>` skeleton plus its `data-*`
//! attributes.

use ctxk_core::KnowledgeItem;
use time::format_description::well_known::Rfc3339;

/// Emit a `<section>` for `item`, suitable for appending to a vault file's
/// `<body>`. The returned string ends with `\n`.
pub fn emit_section(item: &KnowledgeItem) -> String {
    let mut s = String::with_capacity(item.body_html.len() + 512);
    s.push_str("<section");
    push_attr(&mut s, "data-knowledge-id", &item.id);
    push_attr(&mut s, "data-knowledge-type", item.knowledge_type.as_str());
    push_attr(&mut s, "data-scope", item.scope.as_str());
    push_attr(&mut s, "data-confidence", &format!("{:.3}", item.confidence));
    push_attr(&mut s, "data-source-type", item.source_type.as_str());
    push_attr(&mut s, "data-status", item.status.as_str());
    push_attr(&mut s, "data-stability", item.stability.as_str());
    push_attr(
        &mut s,
        "data-created",
        &item.created.format(&Rfc3339).unwrap_or_default(),
    );
    push_attr(
        &mut s,
        "data-modified",
        &item.modified.format(&Rfc3339).unwrap_or_default(),
    );
    if let Some(d) = &item.valid_from {
        push_attr(&mut s, "data-valid-from", &d.format(&Rfc3339).unwrap_or_default());
    } else {
        push_attr(&mut s, "data-valid-from", "");
    }
    if let Some(d) = &item.valid_until {
        push_attr(&mut s, "data-valid-until", &d.format(&Rfc3339).unwrap_or_default());
    } else {
        push_attr(&mut s, "data-valid-until", "");
    }
    if let Some(d) = &item.domain {
        push_attr(&mut s, "data-domain", d);
    }
    if !item.tags.is_empty() {
        push_attr(&mut s, "data-tags", &item.tags.join(" "));
    }
    if let Some(ck) = &item.claim_key {
        push_attr(&mut s, "data-claim-key", ck);
    }
    if let Some(p) = &item.defined_path {
        push_attr(&mut s, "data-path", p);
    }
    if let (Some(s_line), Some(e_line)) = (item.defined_start_line, item.defined_end_line) {
        push_attr(&mut s, "data-defined-at", &format!("{}-{}", s_line, e_line));
    }
    s.push_str(">\n");

    s.push_str(&item.body_html);
    if !item.body_html.ends_with('\n') {
        s.push('\n');
    }

    if !item.relations.is_empty() {
        s.push_str("  <footer class=\"ctxk-meta\">\n");
        for rel in &item.relations {
            s.push_str(&format!(
                "    <span data-rel=\"{}\" data-target=\"{}\"></span>\n",
                escape_attr(&rel.rel),
                escape_attr(&rel.target)
            ));
        }
        s.push_str("  </footer>\n");
    }

    s.push_str("</section>\n");
    s
}

fn push_attr(buf: &mut String, name: &str, value: &str) {
    buf.push(' ');
    buf.push_str(name);
    buf.push_str("=\"");
    buf.push_str(&escape_attr(value));
    buf.push('"');
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
