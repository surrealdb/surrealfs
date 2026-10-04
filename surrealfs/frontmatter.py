"""Frontmatter parsing and link extraction for SurrealFS.

Extracts YAML/TOML frontmatter into queryable dictionary metadata and
discovers markdown links, wikilinks, and file references.
"""

from __future__ import annotations

import posixpath
import re
import tomllib
from typing import Any

from . import paths

__all__ = [
    "extract_markdown_links",
    "parse_frontmatter",
    "resolve_link",
]


def _coerce_scalar(val: str) -> Any:
    val = val.strip()
    if not val:
        return ""
    if val.lower() in ("true", "yes", "on"):
        return True
    if val.lower() in ("false", "no", "off"):
        return False
    if val.lower() in ("null", "none", "~"):
        return None
    if (val.startswith('"') and val.endswith('"')) or (
        val.startswith("'") and val.endswith("'")
    ):
        return val[1:-1]
    try:
        if "." in val:
            return float(val)
        return int(val)
    except ValueError:
        return val


def _parse_yaml_subset(fm_text: str) -> dict[str, Any]:
    """Parse a clean subset of YAML commonly used in frontmatter."""
    data: dict[str, Any] = {}
    current_key: str | None = None
    current_list: list[Any] | None = None

    for raw_line in fm_text.splitlines():
        # Remove comments
        line = raw_line.split("#", 1)[0].rstrip()
        if not line.strip():
            continue

        # Bullet list item under current_key
        bullet_m = re.match(r"^\s*-\s+(.*)$", line)
        if bullet_m and current_key:
            item_val = _coerce_scalar(bullet_m.group(1))
            if current_list is None:
                current_list = []
                data[current_key] = current_list
            current_list.append(item_val)
            continue

        # Key-value pair
        kv_m = re.match(r"^([A-Za-z0-9_-]+)\s*:\s*(.*)$", line)
        if kv_m:
            current_key = kv_m.group(1).strip()
            val_part = kv_m.group(2).strip()
            current_list = None

            if not val_part:
                # Might be followed by list items
                data[current_key] = None
            elif val_part.startswith("[") and val_part.endswith("]"):
                # Inline list: [a, b, c]
                inner = val_part[1:-1].strip()
                if not inner:
                    data[current_key] = []
                else:
                    items = [_coerce_scalar(x) for x in inner.split(",") if x.strip()]
                    data[current_key] = items
            else:
                data[current_key] = _coerce_scalar(val_part)

    return data


def parse_frontmatter(text: str) -> tuple[dict[str, Any] | None, str]:
    """Extract YAML or TOML frontmatter from text if present.

    Returns (metadata_dict, remaining_body). If no frontmatter is found,
    returns (None, text).
    """
    if text.startswith("+++\n") or text.startswith("+++\r\n"):
        parts = re.split(r"^\+\+\+\s*$", text[4:], maxsplit=1, flags=re.MULTILINE)
        if len(parts) == 2:
            try:
                meta = tomllib.loads(parts[0])
                body = parts[1].lstrip("\r\n")
                return meta, body
            except Exception:
                pass

    if text.startswith("---\n") or text.startswith("---\r\n"):
        parts = re.split(r"^---\s*$", text[4:], maxsplit=1, flags=re.MULTILINE)
        if len(parts) == 2:
            try:
                meta = _parse_yaml_subset(parts[0])
                body = parts[1].lstrip("\r\n")
                return meta, body
            except Exception:
                pass

    return None, text


def extract_markdown_links(text: str) -> list[str]:
    """Extract file link targets from markdown links and wikilinks."""
    targets: list[str] = []
    seen: set[str] = set()

    # Standard markdown links: [label](target)
    md_link_re = re.compile(r"\[(?:[^\]]*)\]\(([^)]+)\)")
    for match in md_link_re.finditer(text):
        target = match.group(1).strip()
        # Filter out web links, anchors, and mailto
        if target.startswith(("http://", "https://", "mailto:", "#")) or not target:
            continue
        # Strip trailing fragment/anchor or query
        target = target.split("#", 1)[0].split("?", 1)[0]
        if target and target not in seen:
            seen.add(target)
            targets.append(target)

    # Wikilinks: [[target]] or [[target|label]]
    wiki_re = re.compile(r"\[\[([^\]|]+)(?:\|[^\]]+)?\]\]")
    for match in wiki_re.finditer(text):
        target = match.group(1).strip()
        if target and target not in seen:
            seen.add(target)
            targets.append(target)

    return targets


def resolve_link(source_path: str, target: str) -> str:
    """Resolve a link target relative to source_path."""
    if target.startswith("/"):
        return paths.normalize(target)

    # Parent directory of source_path
    parent_dir = posixpath.dirname(paths.normalize(source_path))
    if not parent_dir or parent_dir == ".":
        parent_dir = "/"
    resolved = posixpath.normpath(posixpath.join(parent_dir, target))
    return paths.normalize(resolved)
