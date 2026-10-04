//! Path normalization and glob matching utilities.

pub fn normalize_path(path: &str) -> String {
    let clean = path.trim();
    if clean.is_empty() || clean == "/" {
        return "/".to_string();
    }

    let is_absolute = clean.starts_with('/');
    let mut parts = Vec::new();

    for segment in clean.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if !parts.is_empty() {
                parts.pop();
            }
        } else {
            parts.push(segment);
        }
    }

    if is_absolute {
        if parts.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", parts.join("/"))
        }
    } else {
        format!("/{}", parts.join("/"))
    }
}

pub fn parent_path(path: &str) -> String {
    let norm = normalize_path(path);
    if norm == "/" {
        return "/".to_string();
    }
    let parts: Vec<&str> = norm.trim_start_matches('/').split('/').collect();
    if parts.len() <= 1 {
        "/".to_string()
    } else {
        format!("/{}", parts[..parts.len() - 1].join("/"))
    }
}

pub fn filename(path: &str) -> String {
    let norm = normalize_path(path);
    if norm == "/" {
        return String::new();
    }
    norm.split('/').next_back().unwrap_or("").to_string()
}

pub fn glob_to_regex(glob: &str) -> String {
    let mut out = String::from("^");
    let mut chars = glob.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        out.push_str("(?:.*/)?");
                    } else {
                        out.push_str(".*");
                    }
                } else {
                    out.push_str("[^/]*");
                }
            }
            '?' => out.push_str("[^/]"),
            '.' | '(' | ')' | '+' | '|' | '^' | '$' | '@' | '%' | '[' | ']' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push('$');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize_path(""), "/");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("/a/b/c"), "/a/b/c");
        assert_eq!(normalize_path("/a/b/../c"), "/a/c");
        assert_eq!(normalize_path("a/b/c/"), "/a/b/c");
    }

    #[test]
    fn test_parent_and_filename() {
        assert_eq!(parent_path("/a/b/c.txt"), "/a/b");
        assert_eq!(filename("/a/b/c.txt"), "c.txt");
        assert_eq!(parent_path("/a"), "/");
        assert_eq!(filename("/a"), "a");
    }

    #[test]
    fn test_glob_to_regex() {
        let re = regex::Regex::new(&glob_to_regex("*.md")).unwrap();
        assert!(re.is_match("test.md"));
        assert!(!re.is_match("a/test.md"));

        let re2 = regex::Regex::new(&glob_to_regex("**/*.md")).unwrap();
        assert!(re2.is_match("a/b/test.md"));
    }
}
