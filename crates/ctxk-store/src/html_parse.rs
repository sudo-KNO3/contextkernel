//! HTML → KnowledgeItem parsing.
//!
//! Vault files are normal HTML5 documents whose body contains a flat
//! sequence of `<section data-knowledge-id=…>` elements. Each becomes one
//! [`KnowledgeItem`]. Relations live inside the section as
//! `<span data-rel data-target></span>` so they don't break rendering.

use ctxk_core::{
    KnowledgeItem, KnowledgeType, Relation, Result, Scope, SourceType, Stability, Status,
};
use scraper::{Html, Selector};
use std::path::Path;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Parse every knowledge `<section>` in `path`, in document order.
pub fn parse_file(path: &Path) -> Result<Vec<KnowledgeItem>> {
    let raw = std::fs::read_to_string(path)?;
    parse_str(&raw)
}

pub fn parse_str(raw: &str) -> Result<Vec<KnowledgeItem>> {
    let doc = Html::parse_document(raw);
    let sel = Selector::parse("section[data-knowledge-id]").unwrap();
    let mut out = Vec::new();
    for el in doc.select(&sel) {
        out.push(parse_section(el)?);
    }
    Ok(out)
}

fn parse_section(el: scraper::ElementRef<'_>) -> Result<KnowledgeItem> {
    let val = el.value();
    let attr = |name: &str| val.attr(name).map(str::to_string);

    let id = attr("data-knowledge-id").ok_or_else(|| {
        ctxk_core::Error::Parse("section missing data-knowledge-id".into())
    })?;

    let knowledge_type = attr("data-knowledge-type")
        .map(|s| KnowledgeType::parse(&s))
        .unwrap_or(KnowledgeType::Fact);
    let scope = attr("data-scope")
        .map(|s| Scope::parse(&s))
        .unwrap_or(Scope::User);
    let confidence = attr("data-confidence")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let source_type = attr("data-source-type")
        .map(|s| SourceType::parse(&s))
        .unwrap_or(SourceType::User);
    let status = attr("data-status")
        .map(|s| Status::parse(&s))
        .unwrap_or(Status::Active);
    let stability = attr("data-stability")
        .map(|s| Stability::parse(&s))
        .unwrap_or(Stability::MediumTerm);

    let now = OffsetDateTime::now_utc();
    let created = attr("data-created")
        .and_then(|s| OffsetDateTime::parse(&s, &Rfc3339).ok())
        .unwrap_or(now);
    let modified = attr("data-modified")
        .and_then(|s| OffsetDateTime::parse(&s, &Rfc3339).ok())
        .unwrap_or(now);
    let valid_from = attr("data-valid-from").and_then(|s| {
        if s.is_empty() {
            None
        } else {
            // Accept either RFC3339 or bare YYYY-MM-DD
            OffsetDateTime::parse(&s, &Rfc3339).ok().or_else(|| {
                parse_date_only(&s)
            })
        }
    });
    let valid_until = attr("data-valid-until").and_then(|s| {
        if s.is_empty() {
            None
        } else {
            OffsetDateTime::parse(&s, &Rfc3339).ok().or_else(|| parse_date_only(&s))
        }
    });

    let domain = attr("data-domain").filter(|s| !s.is_empty());
    let tags: Vec<String> = attr("data-tags")
        .unwrap_or_default()
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();
    let claim_key = attr("data-claim-key").filter(|s| !s.is_empty());

    // Code-aware attrs (Phase 3)
    let defined_path = attr("data-path").filter(|s| !s.is_empty());
    let (defined_start_line, defined_end_line) = parse_lines(attr("data-defined-at").as_deref());

    // Title = text of the first h1/h2/h3 within the section.
    let title_sel = Selector::parse("h1, h2, h3, h4").unwrap();
    let title = el
        .select(&title_sel)
        .next()
        .map(|t| t.text().collect::<Vec<_>>().concat().trim().to_string())
        .unwrap_or_default();

    // Relations from <span data-rel data-target>
    let rel_sel = Selector::parse("[data-rel][data-target]").unwrap();
    let relations: Vec<Relation> = el
        .select(&rel_sel)
        .filter_map(|s| {
            let r = s.value().attr("data-rel")?;
            let t = s.value().attr("data-target")?;
            Some(Relation {
                rel: r.to_string(),
                target: t.to_string(),
            })
        })
        .collect();

    // Plain-text body for FTS + a verbatim inner HTML for round-trip.
    let body_html = el.inner_html();
    let body_text = el
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    Ok(KnowledgeItem {
        id,
        knowledge_type,
        scope,
        confidence,
        source_type,
        status,
        stability,
        created,
        modified,
        valid_from,
        valid_until,
        domain,
        tags,
        title,
        body_text,
        body_html,
        relations,
        claim_key,
        defined_path,
        defined_start_line,
        defined_end_line,
    })
}

fn parse_lines(s: Option<&str>) -> (Option<usize>, Option<usize>) {
    let s = match s {
        Some(s) if !s.is_empty() => s,
        _ => return (None, None),
    };
    if let Some((a, b)) = s.split_once('-') {
        (a.trim().parse().ok(), b.trim().parse().ok())
    } else if let Ok(n) = s.trim().parse() {
        (Some(n), Some(n))
    } else {
        (None, None)
    }
}

fn parse_date_only(s: &str) -> Option<OffsetDateTime> {
    // Treat bare YYYY-MM-DD as midnight UTC.
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    let date = time::Date::parse(s, fmt).ok()?;
    Some(date.midnight().assume_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_section() {
        let html = r#"<!doctype html><html><body>
            <section data-knowledge-id="01HZX">
              <h3>Hello</h3>
              <p>World</p>
            </section>
        </body></html>"#;
        let items = parse_str(html).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "01HZX");
        assert_eq!(items[0].title, "Hello");
        assert!(items[0].body_text.contains("World"));
    }

    #[test]
    fn parses_all_metadata() {
        let html = r#"<!doctype html><html><body>
            <section data-knowledge-id="01ABC"
                     data-knowledge-type="constraint"
                     data-scope="project"
                     data-confidence="0.9"
                     data-source-type="user"
                     data-status="active"
                     data-stability="long-term"
                     data-domain="aermod"
                     data-tags="grid units">
              <h3>Use meters</h3>
              <p>All grids in m.</p>
              <span data-rel="cites" data-target="01XYZ"></span>
            </section>
        </body></html>"#;
        let items = parse_str(html).unwrap();
        let it = &items[0];
        assert_eq!(it.knowledge_type, KnowledgeType::Constraint);
        assert_eq!(it.scope, Scope::Project);
        assert!((it.confidence - 0.9).abs() < 1e-9);
        assert_eq!(it.stability, Stability::LongTerm);
        assert_eq!(it.domain.as_deref(), Some("aermod"));
        assert_eq!(it.tags, vec!["grid", "units"]);
        assert_eq!(it.relations.len(), 1);
        assert_eq!(it.relations[0].rel, "cites");
        assert_eq!(it.relations[0].target, "01XYZ");
    }
}
