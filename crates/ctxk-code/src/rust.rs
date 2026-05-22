//! Rust source-file → CodeSymbol extraction via tree-sitter.

use crate::{CodeSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

/// Parse one Rust source string, return all top-level symbols. `file_path`
/// is the POSIX path relative to the project root, embedded into each
/// symbol verbatim.
pub fn parse_rust(src: &str, file_path: &str) -> Vec<CodeSymbol> {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_rust::language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let root = tree.root_node();

    // Per-file imports (`use` paths) and per-symbol call lists.
    let imports = collect_imports(root, src);

    let mut out: Vec<CodeSymbol> = Vec::new();
    walk_top_level(root, src, file_path, &imports, &mut out, &[]);
    out
}

fn walk_top_level(
    node: Node,
    src: &str,
    file_path: &str,
    imports: &[String],
    out: &mut Vec<CodeSymbol>,
    path_stack: &[String],
) {
    let mut cur = node.walk();
    for child in node.named_children(&mut cur) {
        match child.kind() {
            "function_item" => {
                if let Some(sym) = function_symbol(child, src, file_path, imports, path_stack) {
                    out.push(sym);
                }
            }
            "struct_item" => {
                if let Some(sym) = simple_symbol(child, src, file_path, SymbolKind::Struct, "name", imports, path_stack) {
                    out.push(sym);
                }
            }
            "trait_item" => {
                if let Some(sym) = simple_symbol(child, src, file_path, SymbolKind::Trait, "name", imports, path_stack) {
                    out.push(sym);
                }
            }
            "enum_item" => {
                if let Some(sym) = simple_symbol(child, src, file_path, SymbolKind::Enum, "name", imports, path_stack) {
                    out.push(sym);
                }
            }
            "impl_item" => {
                // Emit one symbol for the impl block itself, then recurse into
                // its body to surface the methods.
                let type_name = field_text(child, "type", src).unwrap_or_else(|| "anonymous".into());
                let trait_name = field_text(child, "trait", src);
                let label = match &trait_name {
                    Some(t) => format!("impl {} for {}", t, type_name),
                    None => format!("impl {}", type_name),
                };
                let (start, end) = line_range(child);
                let body = node_text(child, src).to_string();
                let qname = qualified(path_stack, &label);
                out.push(CodeSymbol {
                    name: label.clone(),
                    qualified_name: qname,
                    kind: SymbolKind::Impl,
                    file_path: file_path.to_string(),
                    start_line: start,
                    end_line: end,
                    body,
                    calls: collect_calls(child, src),
                    imports: imports.to_vec(),
                    language: "rust".into(),
                });
                // Recurse — methods inside the impl get their own items.
                let mut child_stack = path_stack.to_vec();
                child_stack.push(label);
                if let Some(decl_list) = child.child_by_field_name("body") {
                    walk_top_level(decl_list, src, file_path, imports, out, &child_stack);
                }
            }
            "mod_item" => {
                let name = field_text(child, "name", src).unwrap_or_default();
                let (start, end) = line_range(child);
                let body = node_text(child, src).to_string();
                let qname = qualified(path_stack, &name);
                out.push(CodeSymbol {
                    name: name.clone(),
                    qualified_name: qname,
                    kind: SymbolKind::Module,
                    file_path: file_path.to_string(),
                    start_line: start,
                    end_line: end,
                    body,
                    calls: Vec::new(),
                    imports: imports.to_vec(),
                    language: "rust".into(),
                });
                let mut child_stack = path_stack.to_vec();
                child_stack.push(name);
                if let Some(decl_list) = child.child_by_field_name("body") {
                    walk_top_level(decl_list, src, file_path, imports, out, &child_stack);
                }
            }
            _ => {}
        }
    }
}

fn function_symbol(
    node: Node,
    src: &str,
    file_path: &str,
    imports: &[String],
    path_stack: &[String],
) -> Option<CodeSymbol> {
    let name = field_text(node, "name", src)?;
    let (start, end) = line_range(node);
    let body = node_text(node, src).to_string();
    let calls = collect_calls(node, src);
    let qname = qualified(path_stack, &name);
    Some(CodeSymbol {
        name,
        qualified_name: qname,
        kind: SymbolKind::Function,
        file_path: file_path.to_string(),
        start_line: start,
        end_line: end,
        body,
        calls,
        imports: imports.to_vec(),
        language: "rust".into(),
    })
}

fn simple_symbol(
    node: Node,
    src: &str,
    file_path: &str,
    kind: SymbolKind,
    name_field: &str,
    imports: &[String],
    path_stack: &[String],
) -> Option<CodeSymbol> {
    let name = field_text(node, name_field, src)?;
    let (start, end) = line_range(node);
    let body = node_text(node, src).to_string();
    let qname = qualified(path_stack, &name);
    Some(CodeSymbol {
        name,
        qualified_name: qname,
        kind,
        file_path: file_path.to_string(),
        start_line: start,
        end_line: end,
        body,
        calls: Vec::new(),
        imports: imports.to_vec(),
        language: "rust".into(),
    })
}

fn collect_imports(root: Node, src: &str) -> Vec<String> {
    // Walk the whole tree for `use_declaration` nodes; flatten paths.
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "use_declaration" {
            // Just grab the raw text after `use ` to the trailing semicolon.
            let txt = node_text(n, src).trim_end_matches(';').trim();
            let path = txt.strip_prefix("use ").unwrap_or(txt).trim();
            // Strip visibility prefixes.
            let path = path
                .strip_prefix("pub ")
                .or_else(|| path.strip_prefix("pub(crate) "))
                .or_else(|| path.strip_prefix("pub(super) "))
                .unwrap_or(path)
                .trim();
            if !path.is_empty() {
                out.push(path.to_string());
            }
            continue;
        }
        let mut cur = n.walk();
        for c in n.named_children(&mut cur) {
            stack.push(c);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn collect_calls(node: Node, src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        // tree-sitter-rust kinds: `call_expression` for foo(), `macro_invocation`
        // for foo!(). Method calls (`x.foo()`) appear as call_expression with
        // a field_expression in `function`.
        match n.kind() {
            "call_expression" => {
                if let Some(callee) = n.child_by_field_name("function") {
                    if let Some(name) = call_target_name(callee, src) {
                        out.push(name);
                    }
                }
            }
            "macro_invocation" => {
                if let Some(name_node) = n.child_by_field_name("macro") {
                    let name = node_text(name_node, src).to_string();
                    if !name.is_empty() {
                        out.push(format!("{}!", name));
                    }
                }
            }
            _ => {}
        }
        let mut cur = n.walk();
        for c in n.named_children(&mut cur) {
            stack.push(c);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Best-effort extraction of a callable name from a tree-sitter "function"
/// child node. Handles bare idents, `path::ident`, and `obj.method`.
fn call_target_name(node: Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(node_text(node, src).to_string()),
        "scoped_identifier" => {
            // Use the trailing name component; record full path too via fallback.
            node.child_by_field_name("name")
                .map(|n| node_text(n, src).to_string())
                .or_else(|| Some(node_text(node, src).to_string()))
        }
        "field_expression" => node
            .child_by_field_name("field")
            .map(|n| node_text(n, src).to_string()),
        "generic_function" | "type" => node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("type"))
            .and_then(|n| call_target_name(n, src)),
        _ => None,
    }
}

fn qualified(path_stack: &[String], name: &str) -> String {
    if path_stack.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", path_stack.join("::"), name)
    }
}

fn field_text(node: Node, field: &str, src: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(n, src).to_string())
}

fn node_text<'a>(node: Node, src: &'a str) -> &'a str {
    let r = node.byte_range();
    let end = r.end.min(src.len());
    let start = r.start.min(end);
    &src[start..end]
}

fn line_range(node: Node) -> (usize, usize) {
    let s = node.start_position().row + 1;
    let e = node.end_position().row + 1;
    (s, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_functions_and_calls() {
        let src = r#"
fn outer() {
    inner();
    println!("hello");
}

fn inner() -> i32 { 42 }

struct Foo { x: i32 }

impl Foo {
    fn bar(&self) {
        outer();
    }
}

use std::fs;
use std::path::{Path, PathBuf};
"#;
        let syms = parse_rust(src, "src/lib.rs");
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"outer"));
        assert!(names.contains(&"inner"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"bar"));
        // outer() calls inner and println! macro
        let outer = syms.iter().find(|s| s.name == "outer").unwrap();
        assert!(outer.calls.iter().any(|c| c == "inner"));
        assert!(outer.calls.iter().any(|c| c == "println!"));
        // Imports collected
        let some_sym = syms.first().unwrap();
        assert!(some_sym.imports.iter().any(|i| i.contains("std::fs")));
    }
}
