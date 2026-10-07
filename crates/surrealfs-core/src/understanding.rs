use crate::models::{CodeSymbol, PackResult, PackedBlock};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedMeta {
    pub content_type: String,
    pub language: Option<String>,
    pub encoding: String,
    pub is_generated: bool,
}

static RE_SHEBANG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#!\s*(?:/usr/bin/env\s+)?([a-zA-Z0-9_\-]+)").unwrap());

static RE_RS_FN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*(?:pub(?:\([^\)]+\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern(?:\s+[^\{]+)?\s+)?fn\s+([a-zA-Z0-9_]+)\s*(?:<[^>]+>)?\s*\(([^\)]*)\)(?:\s*->\s*([^\{;]+))?)").unwrap()
});

static RE_RS_STRUCT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?:\s*(?:pub(?:\([^\)]+\))?\s+)?(?:struct|enum|trait|union)\s+([a-zA-Z0-9_]+))",
    )
    .unwrap()
});

static RE_PY_DEF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?:\s*(?:async\s+)?def\s+([a-zA-Z0-9_]+)\s*\(([^\)]*)\)(?:\s*->\s*([^:]+))?:)",
    )
    .unwrap()
});

static RE_PY_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(?:\s*class\s+([a-zA-Z0-9_]+)(?:\([^\)]*\))?:)").unwrap());

static RE_JS_FN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+([a-zA-Z0-9_]+)\s*\(([^\)]*)\))").unwrap()
});

static RE_JS_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*(?:export\s+)?(?:default\s+)?class\s+([a-zA-Z0-9_]+))").unwrap()
});

static RE_JS_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*(?:export\s+)?(?:interface|type)\s+([a-zA-Z0-9_]+))").unwrap()
});

static RE_GO_FUNC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*func\s+(?:\([^\)]+\)\s+)?([a-zA-Z0-9_]+)\s*\(([^\)]*)\))").unwrap()
});

static RE_GO_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?:\s*type\s+([a-zA-Z0-9_]+)\s+(?:struct|interface))").unwrap()
});

/// Sniffs content type, programming/natural language, encoding, and whether the file is generated.
pub fn detect_type_and_language(path: &str, data: &[u8]) -> DetectedMeta {
    let lower_path = path.to_lowercase();
    let is_generated = is_generated_file(&lower_path);

    // 1. Magic bytes sniffing
    if data.starts_with(b"%PDF-") {
        return DetectedMeta {
            content_type: "application/pdf".into(),
            language: Some("pdf".into()),
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return DetectedMeta {
            content_type: "image/png".into(),
            language: None,
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"\xFF\xD8\xFF") {
        return DetectedMeta {
            content_type: "image/jpeg".into(),
            language: None,
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        return DetectedMeta {
            content_type: "image/gif".into(),
            language: None,
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"PK\x03\x04") {
        return DetectedMeta {
            content_type: "application/zip".into(),
            language: None,
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"\x1F\x8B") {
        return DetectedMeta {
            content_type: "application/gzip".into(),
            language: None,
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"SQLite format 3\0") {
        return DetectedMeta {
            content_type: "application/vnd.sqlite3".into(),
            language: Some("sqlite".into()),
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"PAR1") {
        return DetectedMeta {
            content_type: "application/vnd.apache.parquet".into(),
            language: Some("parquet".into()),
            encoding: "binary".into(),
            is_generated,
        };
    }
    if data.starts_with(b"\0asm") {
        return DetectedMeta {
            content_type: "application/wasm".into(),
            language: Some("wasm".into()),
            encoding: "binary".into(),
            is_generated,
        };
    }

    // 2. Text heuristics & shebang
    let text = std::str::from_utf8(data);
    let (encoding, is_text) = match text {
        Ok(_) => ("utf-8".to_string(), true),
        Err(_) => ("binary".to_string(), false),
    };

    if !is_text {
        return DetectedMeta {
            content_type: "application/octet-stream".into(),
            language: None,
            encoding,
            is_generated,
        };
    }

    let text_content = text.unwrap_or_default();

    // Check shebang
    if text_content.starts_with("#!") {
        if let Some(first_line) = text_content.lines().next() {
            if let Some(caps) = RE_SHEBANG.captures(first_line) {
                let cmd = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                match cmd {
                    "python" | "python3" => {
                        return DetectedMeta {
                            content_type: "text/x-python".into(),
                            language: Some("python".into()),
                            encoding,
                            is_generated,
                        };
                    }
                    "node" | "nodejs" | "deno" | "bun" => {
                        return DetectedMeta {
                            content_type: "application/javascript".into(),
                            language: Some("javascript".into()),
                            encoding,
                            is_generated,
                        };
                    }
                    "sh" | "bash" | "zsh" => {
                        return DetectedMeta {
                            content_type: "text/x-shellscript".into(),
                            language: Some("shell".into()),
                            encoding,
                            is_generated,
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    // 3. Extension fallback
    let ext = lower_path.rsplit('.').next().unwrap_or("");
    let (ct, lang) = match ext {
        "rs" => ("text/x-rust", Some("rust")),
        "py" => ("text/x-python", Some("python")),
        "ts" => ("application/typescript", Some("typescript")),
        "tsx" => ("application/typescript", Some("typescript")),
        "js" | "mjs" | "cjs" => ("application/javascript", Some("javascript")),
        "jsx" => ("application/javascript", Some("javascript")),
        "go" => ("text/x-go", Some("go")),
        "c" | "h" => ("text/x-c", Some("c")),
        "cpp" | "cc" | "cxx" | "hpp" => ("text/x-c++", Some("cpp")),
        "java" => ("text/x-java", Some("java")),
        "swift" => ("text/x-swift", Some("swift")),
        "sql" => ("application/sql", Some("sql")),
        "surql" => ("application/sql", Some("surrealql")),
        "md" | "markdown" => ("text/markdown", Some("markdown")),
        "json" => ("application/json", Some("json")),
        "yaml" | "yml" => ("application/yaml", Some("yaml")),
        "toml" => ("application/toml", Some("toml")),
        "html" | "htm" => ("text/html", Some("html")),
        "css" | "scss" => ("text/css", Some("css")),
        "sh" | "bash" | "zsh" => ("text/x-shellscript", Some("shell")),
        "txt" => ("text/plain", Some("text")),
        _ => ("text/plain", None),
    };

    DetectedMeta {
        content_type: ct.into(),
        language: lang.map(|s| s.into()),
        encoding,
        is_generated,
    }
}

fn is_generated_file(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    if filename.ends_with(".min.js")
        || filename.ends_with(".min.css")
        || filename.ends_with(".map")
        || filename == "cargo.lock"
        || filename == "package-lock.json"
        || filename == "pnpm-lock.yaml"
        || filename == "yarn.lock"
        || filename == "poetry.lock"
        || filename == "bun.lockb"
    {
        return true;
    }

    path.contains("/dist/")
        || path.contains("/target/")
        || path.contains("/node_modules/")
        || path.contains("/build/")
        || path.contains("/.git/")
}

/// Extracts symbols (functions, structs, classes, methods, traits) from source code.
pub fn extract_symbols(path: &str, content: &str, language: &str) -> Vec<CodeSymbol> {
    let mut symbols = Vec::new();
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path);

    let lines: Vec<&str> = content.lines().collect();

    match language {
        "rust" => {
            for (idx, line) in lines.iter().enumerate() {
                let line_num = idx + 1;
                if let Some(caps) = RE_RS_FN.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}::{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: "function".to_string(),
                        language: "rust".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "///"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                } else if let Some(caps) = RE_RS_STRUCT.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}::{}", stem, name);
                    let kind = if line.contains("struct ") {
                        "struct"
                    } else if line.contains("enum ") {
                        "enum"
                    } else if line.contains("trait ") {
                        "trait"
                    } else {
                        "type"
                    };
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: kind.to_string(),
                        language: "rust".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "///"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                }
            }
        }
        "python" => {
            for (idx, line) in lines.iter().enumerate() {
                let line_num = idx + 1;
                if let Some(caps) = RE_PY_DEF.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let is_method = line.starts_with("    ") || line.starts_with('\t');
                    let kind = if is_method { "method" } else { "function" };
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: kind.to_string(),
                        language: "python".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_python_doc(&lines, idx),
                        line_start: line_num,
                        line_end: find_python_block_end(&lines, idx),
                    });
                } else if let Some(caps) = RE_PY_CLASS.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: "class".to_string(),
                        language: "python".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_python_doc(&lines, idx),
                        line_start: line_num,
                        line_end: find_python_block_end(&lines, idx),
                    });
                }
            }
        }
        "typescript" | "javascript" => {
            for (idx, line) in lines.iter().enumerate() {
                let line_num = idx + 1;
                if let Some(caps) = RE_JS_FN.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: "function".to_string(),
                        language: language.to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "//"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                } else if let Some(caps) = RE_JS_CLASS.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: "class".to_string(),
                        language: language.to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "//"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                } else if let Some(caps) = RE_JS_TYPE.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let qualified = format!("{}.{}", stem, name);
                    let kind = if line.contains("interface ") {
                        "interface"
                    } else {
                        "type"
                    };
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: kind.to_string(),
                        language: language.to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "//"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                }
            }
        }
        "go" => {
            for (idx, line) in lines.iter().enumerate() {
                let line_num = idx + 1;
                if let Some(caps) = RE_GO_FUNC.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let is_method = line.contains("func (");
                    let kind = if is_method { "method" } else { "function" };
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: kind.to_string(),
                        language: "go".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "//"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                } else if let Some(caps) = RE_GO_TYPE.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                    let kind = if line.contains("struct") {
                        "struct"
                    } else {
                        "interface"
                    };
                    let qualified = format!("{}.{}", stem, name);
                    symbols.push(CodeSymbol {
                        id: None,
                        file_id: None,
                        path: Some(path.to_string()),
                        name: name.to_string(),
                        qualified,
                        kind: kind.to_string(),
                        language: "go".to_string(),
                        signature: Some(line.trim().to_string()),
                        doc: extract_preceding_doc(&lines, idx, "//"),
                        line_start: line_num,
                        line_end: find_block_end(&lines, idx),
                    });
                }
            }
        }
        _ => {}
    }

    symbols
}

fn extract_preceding_doc(lines: &[&str], curr_idx: usize, comment_prefix: &str) -> Option<String> {
    if curr_idx == 0 {
        return None;
    }
    let mut doc_lines = Vec::new();
    let mut i = curr_idx - 1;
    loop {
        let trimmed = lines[i].trim();
        if trimmed.starts_with(comment_prefix) {
            let clean = trimmed.trim_start_matches(comment_prefix).trim();
            doc_lines.push(clean);
            if i == 0 {
                break;
            }
            i -= 1;
        } else {
            break;
        }
    }
    if doc_lines.is_empty() {
        None
    } else {
        doc_lines.reverse();
        Some(doc_lines.join("\n"))
    }
}

fn extract_python_doc(lines: &[&str], curr_idx: usize) -> Option<String> {
    if curr_idx + 1 >= lines.len() {
        return None;
    }
    let next_line = lines[curr_idx + 1].trim();
    if next_line.starts_with("\"\"\"") || next_line.starts_with("'''") {
        let quote = &next_line[..3];
        if next_line.len() > 3 && next_line[3..].contains(quote) {
            let doc = next_line.trim_matches(|c| c == '"' || c == '\'').trim();
            return Some(doc.to_string());
        }
        let mut doc_lines = vec![next_line.trim_start_matches(quote)];
        for line in lines.iter().skip(curr_idx + 2) {
            let t = line.trim();
            if t.contains(quote) {
                let end_part = t.split(quote).next().unwrap_or("");
                doc_lines.push(end_part);
                break;
            }
            doc_lines.push(t);
        }
        Some(doc_lines.join("\n"))
    } else {
        None
    }
}

fn find_block_end(lines: &[&str], start_idx: usize) -> usize {
    let mut brace_count = 0;
    let mut found_open = false;
    for (idx, line) in lines.iter().enumerate().skip(start_idx) {
        for c in line.chars() {
            if c == '{' {
                brace_count += 1;
                found_open = true;
            } else if c == '}' {
                brace_count -= 1;
            }
        }
        if found_open && brace_count <= 0 {
            return idx + 1;
        }
    }
    start_idx + 1
}

fn find_python_block_end(lines: &[&str], start_idx: usize) -> usize {
    let start_line = lines[start_idx];
    let start_indent = start_line.len() - start_line.trim_start().len();
    for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent <= start_indent {
            return idx;
        }
    }
    lines.len()
}

/// Token count estimation: ~4 chars per token average, with whitespace word weighting.
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count();
    let words = text.split_whitespace().count();
    let est = (chars / 4).max(words);
    est.max(1)
}

/// Packs candidate code/text blocks into a single markdown context document within a token budget (§20.6).
pub fn pack_blocks(question: &str, budget: usize, mut candidates: Vec<PackedBlock>) -> PackResult {
    // Sort descending by score
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut used_tokens = estimate_tokens(question) + 30; // budget for question & header
    let mut accepted = Vec::new();
    let mut formatted_parts = Vec::new();

    formatted_parts.push(format!("# Context for: {}\n", question));

    for block in candidates {
        let block_tokens = block.tokens;
        if used_tokens + block_tokens > budget {
            continue;
        }
        used_tokens += block_tokens;
        let ext = block.path.rsplit('.').next().unwrap_or("");
        formatted_parts.push(format!(
            "### `{}:{}-{}`\n```{}\n{}\n```\n",
            block.path, block.line_start, block.line_end, ext, block.content
        ));
        accepted.push(block);
    }

    PackResult {
        question: question.to_string(),
        budget,
        used_tokens,
        blocks: accepted,
        formatted: formatted_parts.join("\n"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtractedEntity {
    pub name: String,
    pub kind: String,
}

static RE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)#([a-zA-Z][a-zA-Z0-9_\-]{1,50})").unwrap());
static RE_TICKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Z]{2,10}-[0-9]{1,6})\b").unwrap());
static RE_MENTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@([a-zA-Z0-9_\.\-]{2,50})").unwrap());

/// Extracts tags and entities (people, companies, systems, tickets) from text (§20.7).
pub fn extract_entities_and_tags(text: &str) -> (Vec<String>, Vec<ExtractedEntity>) {
    let mut tags = Vec::new();
    let mut entities = Vec::new();

    // 1. Tags
    for caps in RE_TAG.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let t = m.as_str().to_lowercase();
            if !tags.contains(&t) {
                tags.push(t.clone());
                entities.push(ExtractedEntity {
                    name: t,
                    kind: "tag".to_string(),
                });
            }
        }
    }

    // 2. Tickets
    for caps in RE_TICKET.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let tick = m.as_str().to_string();
            if !entities.iter().any(|e| e.name == tick) {
                entities.push(ExtractedEntity {
                    name: tick,
                    kind: "ticket".to_string(),
                });
            }
        }
    }

    // 3. Mentions / Persons
    for caps in RE_MENTION.captures_iter(text) {
        if let Some(m) = caps.get(1) {
            let user = m.as_str().to_string();
            if !entities.iter().any(|e| e.name == user) {
                entities.push(ExtractedEntity {
                    name: user,
                    kind: "person".to_string(),
                });
            }
        }
    }

    // 4. Well-known systems and cloud providers
    let known_systems = [
        ("okta", "system"),
        ("aws", "system"),
        ("gcp", "system"),
        ("azure", "system"),
        ("surrealdb", "system"),
        ("redis", "system"),
        ("postgres", "system"),
        ("kafka", "system"),
        ("kubernetes", "system"),
        ("docker", "system"),
        ("github", "system"),
        ("datadog", "system"),
        ("stripe", "company"),
        ("openai", "company"),
        ("anthropic", "company"),
    ];

    let lower_text = text.to_lowercase();
    for &(term, kind) in &known_systems {
        let pattern = format!(r"\b{}\b", term);
        if let Ok(re) = Regex::new(&pattern) {
            if re.is_match(&lower_text)
                && !entities.iter().any(|e| e.name.eq_ignore_ascii_case(term))
            {
                entities.push(ExtractedEntity {
                    name: term.to_string(),
                    kind: kind.to_string(),
                });
            }
        }
    }

    (tags, entities)
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Computes 64-bit SimHash lexical signature of text (§20.8).
pub fn compute_simhash(text: &str) -> u64 {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    if words.is_empty() {
        return 0;
    }

    let mut v = [0i32; 64];

    if words.len() >= 3 {
        for i in 0..=(words.len() - 3) {
            let gram = format!("{} {} {}", words[i], words[i + 1], words[i + 2]);
            let h = fnv1a_64(gram.as_bytes());
            for (bit, item) in v.iter_mut().enumerate() {
                if (h & (1u64 << bit)) != 0 {
                    *item += 1;
                } else {
                    *item -= 1;
                }
            }
        }
    } else {
        for word in &words {
            let h = fnv1a_64(word.as_bytes());
            for (bit, item) in v.iter_mut().enumerate() {
                if (h & (1u64 << bit)) != 0 {
                    *item += 1;
                } else {
                    *item -= 1;
                }
            }
        }
    }

    let mut fingerprint = 0u64;
    for (bit, count) in v.iter().enumerate() {
        if *count > 0 {
            fingerprint |= 1u64 << bit;
        }
    }
    fingerprint
}

/// Computes similarity in range [0.0, 1.0] between two 64-bit SimHash signatures.
pub fn simhash_similarity(h1: u64, h2: u64) -> f64 {
    let dist = (h1 ^ h2).count_ones();
    1.0 - (dist as f64 / 64.0)
}

/// Parse CSV / TSV text into column headers and JSON row objects (§21.6).
pub fn parse_tabular_records(content: &str, limit: usize) -> (Vec<String>, Vec<serde_json::Value>) {
    let lines: Vec<&str> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let delim = if lines[0].contains('\t') { '\t' } else { ',' };

    let headers: Vec<String> = lines[0]
        .split(delim)
        .map(|s| s.trim().trim_matches('"').to_string())
        .collect();

    let mut rows = Vec::new();
    for line in lines.iter().skip(1).take(limit) {
        let cols: Vec<&str> = line
            .split(delim)
            .map(|s| s.trim().trim_matches('"'))
            .collect();
        let mut map = serde_json::Map::new();
        for (i, header) in headers.iter().enumerate() {
            let val_str = cols.get(i).copied().unwrap_or("");
            let val = if let Ok(int_val) = val_str.parse::<i64>() {
                serde_json::Value::Number(int_val.into())
            } else if let Ok(float_val) = val_str.parse::<f64>() {
                if let Some(num) = serde_json::Number::from_f64(float_val) {
                    serde_json::Value::Number(num)
                } else {
                    serde_json::Value::String(val_str.to_string())
                }
            } else if val_str.eq_ignore_ascii_case("true") {
                serde_json::Value::Bool(true)
            } else if val_str.eq_ignore_ascii_case("false") {
                serde_json::Value::Bool(false)
            } else {
                serde_json::Value::String(val_str.to_string())
            };
            map.insert(header.clone(), val);
        }
        rows.push(serde_json::Value::Object(map));
    }

    (headers, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_magic_detection() {
        let pdf = b"%PDF-1.7 ...";
        let meta = detect_type_and_language("/doc.bin", pdf);
        assert_eq!(meta.content_type, "application/pdf");
        assert_eq!(meta.language.as_deref(), Some("pdf"));
        assert_eq!(meta.encoding, "binary");
        assert!(!meta.is_generated);

        let png = b"\x89PNG\r\n\x1a\n\x00\x00";
        let meta = detect_type_and_language("/image.raw", png);
        assert_eq!(meta.content_type, "image/png");
        assert_eq!(meta.encoding, "binary");

        let sqlite = b"SQLite format 3\0\x04\x00";
        let meta = detect_type_and_language("/db.data", sqlite);
        assert_eq!(meta.content_type, "application/vnd.sqlite3");
        assert_eq!(meta.language.as_deref(), Some("sqlite"));
    }

    #[test]
    fn test_shebang_and_extension_detection() {
        let py_script = b"#!/usr/bin/env python3\nprint('hello world')\n";
        let meta = detect_type_and_language("/bin/script", py_script);
        assert_eq!(meta.content_type, "text/x-python");
        assert_eq!(meta.language.as_deref(), Some("python"));
        assert_eq!(meta.encoding, "utf-8");

        let rs_code = b"fn main() {}";
        let meta = detect_type_and_language("/src/main.rs", rs_code);
        assert_eq!(meta.content_type, "text/x-rust");
        assert_eq!(meta.language.as_deref(), Some("rust"));
        assert_eq!(meta.encoding, "utf-8");

        let lockfile = b"[package]\n";
        let meta = detect_type_and_language("/project/Cargo.lock", lockfile);
        assert!(meta.is_generated);

        let min_js = b"function a(){}";
        let meta = detect_type_and_language("/static/bundle.min.js", min_js);
        assert!(meta.is_generated);
    }

    #[test]
    fn test_extract_symbols_rust() {
        let code = r#"
/// A fast user record.
pub struct User {
    pub id: u64,
}

/// Compute greeting for user.
pub async fn greet(user: &User) -> String {
    format!("Hello {}", user.id)
}
"#;
        let symbols = extract_symbols("/src/lib.rs", code, "rust");
        assert_eq!(symbols.len(), 2);

        assert_eq!(symbols[0].name, "User");
        assert_eq!(symbols[0].kind, "struct");
        assert_eq!(symbols[0].qualified, "lib::User");
        assert_eq!(symbols[0].doc.as_deref(), Some("A fast user record."));

        assert_eq!(symbols[1].name, "greet");
        assert_eq!(symbols[1].kind, "function");
        assert_eq!(symbols[1].qualified, "lib::greet");
        assert_eq!(
            symbols[1].doc.as_deref(),
            Some("Compute greeting for user.")
        );
    }

    #[test]
    fn test_extract_symbols_python() {
        let code = r#"
class Engine:
    """Core compute engine."""
    def run(self):
        return True

def standalone(x: int) -> int:
    """Double the input."""
    return x * 2
"#;
        let symbols = extract_symbols("/app/engine.py", code, "python");
        assert_eq!(symbols.len(), 3);

        assert_eq!(symbols[0].name, "Engine");
        assert_eq!(symbols[0].kind, "class");
        assert_eq!(symbols[0].qualified, "engine.Engine");
        assert_eq!(symbols[0].doc.as_deref(), Some("Core compute engine."));

        assert_eq!(symbols[1].name, "run");
        assert_eq!(symbols[1].kind, "method");
        assert_eq!(symbols[1].qualified, "engine.run");

        assert_eq!(symbols[2].name, "standalone");
        assert_eq!(symbols[2].kind, "function");
        assert_eq!(symbols[2].doc.as_deref(), Some("Double the input."));
    }

    #[test]
    fn test_pack_blocks_budget() {
        let b1 = PackedBlock {
            path: "/a.py".to_string(),
            line_start: 1,
            line_end: 5,
            content: "def a(): pass".to_string(),
            tokens: 10,
            score: 0.9,
        };
        let b2 = PackedBlock {
            path: "/b.py".to_string(),
            line_start: 1,
            line_end: 10,
            content: "def b(): pass".to_string(),
            tokens: 500,
            score: 0.5,
        };

        let res = pack_blocks("How does a work?", 100, vec![b1, b2]);
        assert_eq!(res.blocks.len(), 1);
        assert_eq!(res.blocks[0].path, "/a.py");
        assert!(res.used_tokens <= 100);
        assert!(res.formatted.contains("Context for: How does a work?"));
        assert!(res.formatted.contains("### `/a.py:1-5`"));
    }

    #[test]
    fn test_simhash_near_duplicates() {
        let t1 = "SurrealFS provides unified storage with ACID transactions and vector embeddings for AI agents.";
        let t2 = "SurrealFS provides unified storage with ACID transactions and vector embeddings for autonomous agents.";
        let t3 = "An apple a day keeps the doctor away in the sunny garden.";

        let h1 = compute_simhash(t1);
        let h2 = compute_simhash(t2);
        let h3 = compute_simhash(t3);

        assert_eq!(simhash_similarity(h1, h1), 1.0);
        let sim_close = simhash_similarity(h1, h2);
        let sim_far = simhash_similarity(h1, h3);

        assert!(sim_close > 0.80, "Expected close similarity: {}", sim_close);
        assert!(sim_far < 0.70, "Expected far similarity: {}", sim_far);
    }

    #[test]
    fn test_extract_entities_and_tags() {
        let text = "Check ticket SEC-1042: @alice reported an issue with Okta and AWS authentication. #security #auth";
        let (tags, entities) = extract_entities_and_tags(text);

        assert!(tags.contains(&"security".to_string()));
        assert!(tags.contains(&"auth".to_string()));

        let names: Vec<String> = entities.into_iter().map(|e| e.name).collect();
        assert!(names.contains(&"security".to_string()));
        assert!(names.contains(&"SEC-1042".to_string()));
        assert!(names.contains(&"alice".to_string()));
        assert!(names.contains(&"okta".to_string()));
        assert!(names.contains(&"aws".to_string()));
    }

    #[test]
    fn test_parse_tabular_records() {
        let csv = "id,name,active,score\n1,Alice,true,98.5\n2,Bob,false,85.0\n";
        let (headers, rows) = parse_tabular_records(csv, 10);

        assert_eq!(headers, vec!["id", "name", "active", "score"]);
        assert_eq!(rows.len(), 2);

        let r1 = &rows[0];
        assert_eq!(r1["id"], 1);
        assert_eq!(r1["name"], "Alice");
        assert_eq!(r1["active"], true);
        assert_eq!(r1["score"], 98.5);
    }
}
