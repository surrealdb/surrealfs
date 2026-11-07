use crate::models::BlobChunk;
use fastcdc::FastCDC;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FastCdcConfig {
    pub min_size: usize,
    pub avg_size: usize,
    pub max_size: usize,
}

impl Default for FastCdcConfig {
    fn default() -> Self {
        Self {
            min_size: 256 * 1024,      // 256 KB
            avg_size: 1024 * 1024,     // 1 MB
            max_size: 4 * 1024 * 1024, // 4 MB
        }
    }
}

impl FastCdcConfig {
    pub fn small() -> Self {
        Self {
            min_size: 4 * 1024,  // 4 KB
            avg_size: 16 * 1024, // 16 KB
            max_size: 64 * 1024, // 64 KB
        }
    }
}

/// Hashes and compresses one chunk of a file that starts at `offset`.
pub fn encode_chunk(chunk_bytes: &[u8], offset: u64) -> BlobChunk {
    let hash = blake3::hash(chunk_bytes).to_hex().to_string();
    let chunk_id = format!("b3-{}", hash);

    // Attempt transparent compression with zstd on native targets
    #[cfg(not(target_arch = "wasm32"))]
    let (stored_bytes, codec) = match zstd::encode_all(chunk_bytes, 3) {
        Ok(comp) if comp.len() < chunk_bytes.len() => (comp, "zstd".to_string()),
        _ => (chunk_bytes.to_vec(), "none".to_string()),
    };
    #[cfg(target_arch = "wasm32")]
    let (stored_bytes, codec) = (chunk_bytes.to_vec(), "none".to_string());

    BlobChunk {
        chunk_id,
        offset,
        length: chunk_bytes.len() as u64,
        uncompressed_size: chunk_bytes.len() as u64,
        stored_size: stored_bytes.len() as u64,
        codec,
        data: Some(stored_bytes),
    }
}

/// Chunks raw byte data using FastCDC rolling hash, BLAKE3 content addressing,
/// and zstd transparent compression.
pub fn chunk_data(data: &[u8], config: Option<FastCdcConfig>) -> Vec<BlobChunk> {
    if data.is_empty() {
        return Vec::new();
    }

    let cfg = config.unwrap_or_default();
    FastCDC::new(data, cfg.min_size, cfg.avg_size, cfg.max_size)
        .map(|chunk| {
            encode_chunk(
                &data[chunk.offset..chunk.offset + chunk.length],
                chunk.offset as u64,
            )
        })
        .collect()
}

/// Cuts a byte stream into the chunks [`chunk_data`] would cut from the same bytes, holding at
/// most `2 * max_size` plus the last piece pushed.
///
/// A cut point depends only on the bytes from the start of its chunk, and a chunk is at most
/// `max_size` long. A chunk that is followed by another in the buffer therefore ends where it
/// would in the whole file; only the last chunk in the buffer can still move, so it is kept
/// until more bytes arrive or [`finish`](Self::finish) is called.
pub struct StreamChunker {
    cfg: FastCdcConfig,
    buf: Vec<u8>,
    offset: u64,
}

impl StreamChunker {
    pub fn new(config: Option<FastCdcConfig>) -> Self {
        Self {
            cfg: config.unwrap_or_default(),
            buf: Vec::new(),
            offset: 0,
        }
    }

    /// Add bytes and return the chunks that are now final.
    pub fn push(&mut self, data: &[u8]) -> Vec<BlobChunk> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        while self.buf.len() >= 2 * self.cfg.max_size {
            let cuts: Vec<(usize, usize)> = FastCDC::new(
                &self.buf,
                self.cfg.min_size,
                self.cfg.avg_size,
                self.cfg.max_size,
            )
            .map(|c| (c.offset, c.length))
            .collect();
            let Some((last_offset, _)) = cuts.last().copied() else {
                break;
            };
            for &(offset, length) in &cuts[..cuts.len() - 1] {
                out.push(encode_chunk(
                    &self.buf[offset..offset + length],
                    self.offset + offset as u64,
                ));
            }
            self.offset += last_offset as u64;
            self.buf.drain(..last_offset);
        }
        out
    }

    /// The chunks of the bytes still held, after the last push.
    pub fn finish(&mut self) -> Vec<BlobChunk> {
        let out = chunk_data(&self.buf, Some(self.cfg))
            .into_iter()
            .map(|mut c| {
                c.offset += self.offset;
                c
            })
            .collect();
        self.offset += self.buf.len() as u64;
        self.buf.clear();
        out
    }
}

/// Computes the whole-file Merkle root hash over ordered chunk BLAKE3 identifiers.
pub fn compute_merkle_root(chunk_ids: &[String]) -> String {
    let mut hasher = blake3::Hasher::new();
    for id in chunk_ids {
        hasher.update(id.as_bytes());
    }
    format!("b3-{}", hasher.finalize().to_hex())
}

/// Decompresses chunk bytes according to the blob codec.
pub fn decompress_chunk(data: &[u8], codec: &str) -> Result<Vec<u8>, std::io::Error> {
    match codec {
        #[cfg(not(target_arch = "wasm32"))]
        "zstd" => zstd::decode_all(data),
        #[cfg(target_arch = "wasm32")]
        "zstd" => {
            use std::io::Read;
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(data)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
            let mut out = Vec::new();
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
        _ => Ok(data.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_chunker_matches_one_shot_chunking() {
        let mut data = Vec::new();
        for i in 0..40_000u32 {
            data.extend_from_slice(format!("line {i} {}\n", i.wrapping_mul(2654435761)).as_bytes());
        }
        let whole = chunk_data(&data, Some(FastCdcConfig::small()));
        for piece in [1usize, 7, 4_099, 70_000, data.len()] {
            let mut chunker = StreamChunker::new(Some(FastCdcConfig::small()));
            let mut got = Vec::new();
            for part in data.chunks(piece) {
                got.extend(chunker.push(part));
            }
            got.extend(chunker.finish());
            let ids: Vec<_> = got
                .iter()
                .map(|c| (&c.chunk_id, c.offset, c.length))
                .collect();
            let want: Vec<_> = whole
                .iter()
                .map(|c| (&c.chunk_id, c.offset, c.length))
                .collect();
            assert_eq!(ids, want, "pieces of {piece} bytes");
        }
    }

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

    #[test]
    fn test_fastcdc_chunk_data_and_compression() {
        let mut sample = Vec::new();
        // Repetitive compressible data to test zstd compression
        for i in 0..100_000 {
            sample.extend_from_slice(
                format!("Repeated pattern line {} - payload data\n", i % 10).as_bytes(),
            );
        }

        let chunks = chunk_data(&sample, Some(FastCdcConfig::small()));
        assert!(!chunks.is_empty());

        let mut reassembled = Vec::new();
        for chunk in &chunks {
            assert!(chunk.chunk_id.starts_with("b3-"));
            let decompressed =
                decompress_chunk(chunk.data.as_ref().unwrap(), &chunk.codec).unwrap();
            assert_eq!(decompressed.len(), chunk.length as usize);
            reassembled.extend_from_slice(&decompressed);
        }
        assert_eq!(reassembled, sample);

        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.chunk_id.clone()).collect();
        let merkle = compute_merkle_root(&chunk_ids);
        assert!(merkle.starts_with("b3-"));
    }
}
