/**
 * Path handling utilities for SurrealFS TypeScript SDK.
 */

export function normalize(path: string): string {
  if (!path || path.trim() === "") {
    return "/";
  }
  let p = path.trim().replace(/\\/g, "/");
  if (!p.startsWith("/")) {
    p = "/" + p;
  }
  const segments = p.split("/").filter(Boolean);
  const resolved: string[] = [];

  for (const seg of segments) {
    if (seg === ".") continue;
    if (seg === "..") {
      resolved.pop();
    } else {
      resolved.push(seg);
    }
  }

  const result = "/" + resolved.join("/");
  return result;
}

export function parentPath(path: string): string {
  const norm = normalize(path);
  if (norm === "/") return "/";
  const lastIdx = norm.lastIndexOf("/");
  if (lastIdx <= 0) return "/";
  return norm.slice(0, lastIdx);
}

export function fileName(path: string): string {
  const norm = normalize(path);
  if (norm === "/") return "";
  const lastIdx = norm.lastIndexOf("/");
  return norm.slice(lastIdx + 1);
}

export function globToRegex(glob: string): RegExp {
  let re = "";
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === "*") {
      if (glob[i + 1] === "*") {
        re += ".*";
        i++;
      } else {
        re += "[^/]*";
      }
    } else if (c === "?") {
      re += "[^/]";
    } else if ("()+[]^$.{}\\".includes(c)) {
      re += "\\" + c;
    } else {
      re += c;
    }
  }
  return new RegExp(`^${re}$`);
}
