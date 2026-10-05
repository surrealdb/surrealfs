use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSection {
    pub section_idx: usize,
    pub heading: String,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub source_hash: String,
}

fn sha256_hex(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

/// Split markdown text into hierarchical sections along heading boundaries (#, ##, ###).
pub fn chunk_markdown(text: &str) -> Vec<FileSection> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let lines: Vec<&str> = text.lines().collect();
    let heading_re = Regex::new(r"^(#{1,6})\s+(.+?)\s*#*$").unwrap();

    let mut headings: Vec<(usize, usize, String)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if let Some(caps) = heading_re.captures(line) {
            let level = caps.get(1).unwrap().as_str().len();
            let title = caps.get(2).unwrap().as_str().trim().to_string();
            headings.push((idx, level, title));
        }
    }

    if headings.is_empty() {
        return vec![FileSection {
            section_idx: 0,
            heading: String::new(),
            line_start: 1,
            line_end: lines.len(),
            content: text.to_string(),
            source_hash: sha256_hex(text),
        }];
    }

    let mut sections: Vec<FileSection> = Vec::new();

    // Check for preamble/introduction before the first heading
    let first_h_line = headings[0].0;
    if first_h_line > 0 {
        let preamble_lines = &lines[0..first_h_line];
        let preamble = preamble_lines.join("\n");
        if !preamble.trim().is_empty() {
            sections.push(FileSection {
                section_idx: 0,
                heading: "Overview".to_string(),
                line_start: 1,
                line_end: first_h_line,
                content: preamble.clone(),
                source_hash: sha256_hex(&preamble),
            });
        }
    }

    // Heading hierarchy stack: [(level, title)]
    let mut stack: Vec<(usize, String)> = Vec::new();
    for i in 0..headings.len() {
        let (line_idx, level, ref title) = headings[i];
        let next_line_idx = if i + 1 < headings.len() {
            headings[i + 1].0
        } else {
            lines.len()
        };

        let section_lines = &lines[line_idx..next_line_idx];
        let section_text = section_lines.join("\n");

        while let Some(&(top_level, _)) = stack.last() {
            if top_level >= level {
                stack.pop();
            } else {
                break;
            }
        }
        stack.push((level, title.clone()));
        let heading_path = stack
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join(" > ");

        let section_idx = sections.len();
        sections.push(FileSection {
            section_idx,
            heading: heading_path,
            line_start: line_idx + 1,
            line_end: next_line_idx,
            content: section_text.clone(),
            source_hash: sha256_hex(&section_text),
        });
    }

    sections
}

/// Chunk text into sliding windows of lines with overlap.
pub fn chunk_lines(text: &str, window_size: usize, overlap: usize) -> Vec<FileSection> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    if total_lines <= window_size {
        return vec![FileSection {
            section_idx: 0,
            heading: format!("Lines 1-{}", total_lines),
            line_start: 1,
            line_end: total_lines,
            content: text.to_string(),
            source_hash: sha256_hex(text),
        }];
    }

    let mut sections = Vec::new();
    let step = std::cmp::max(1, window_size.saturating_sub(overlap));
    let mut start = 0;
    let mut idx = 0;

    while start < total_lines {
        let end = std::cmp::min(total_lines, start + window_size);
        let chunk = lines[start..end].join("\n");
        sections.push(FileSection {
            section_idx: idx,
            heading: format!("Lines {}-{}", start + 1, end),
            line_start: start + 1,
            line_end: end,
            content: chunk.clone(),
            source_hash: sha256_hex(&chunk),
        });
        idx += 1;
        if end >= total_lines {
            break;
        }
        start += step;
    }

    sections
}

/// Automatically select and execute the appropriate chunking strategy based on file name.
pub fn chunk_text(text: &str, filename: &str) -> Vec<FileSection> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".markdown") {
        chunk_markdown(text)
    } else {
        let lines_count = text.lines().count();
        if lines_count <= 30 {
            vec![FileSection {
                section_idx: 0,
                heading: if filename.is_empty() {
                    "Document".to_string()
                } else {
                    filename.to_string()
                },
                line_start: 1,
                line_end: std::cmp::max(1, lines_count),
                content: text.to_string(),
                source_hash: sha256_hex(text),
            }]
        } else {
            chunk_lines(text, 50, 10)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_markdown_hierarchy() {
        let md = r#"Preamble documentation here.

# Introduction
This is the intro paragraph.

## Sub-feature A
Details of sub-feature A.

## Sub-feature B
Details of sub-feature B.

# Conclusion
Final thoughts.
"#;
        let sections = chunk_markdown(md);
        assert_eq!(sections.len(), 5);

        assert_eq!(sections[0].heading, "Overview");
        assert_eq!(sections[0].line_start, 1);

        assert_eq!(sections[1].heading, "Introduction");
        assert_eq!(sections[2].heading, "Introduction > Sub-feature A");
        assert_eq!(sections[3].heading, "Introduction > Sub-feature B");
        assert_eq!(sections[4].heading, "Conclusion");
    }

    #[test]
    fn test_chunk_lines_sliding_window() {
        let mut lines = Vec::new();
        for i in 1..=120 {
            lines.push(format!("Line {}", i));
        }
        let text = lines.join("\n");
        let sections = chunk_lines(&text, 50, 10);
        assert!(!sections.is_empty());
        assert_eq!(sections[0].line_start, 1);
        assert_eq!(sections[0].line_end, 50);
        assert_eq!(sections[1].line_start, 41);
        assert_eq!(sections[1].line_end, 90);
    }
}
