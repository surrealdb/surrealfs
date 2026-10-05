use std::collections::HashMap;
use surrealfs_core::{FileEntry, FileLock, SurrealFs, SurrealFsError};

pub const XATTR_OWNER: &str = "user.surrealfs.owner";
pub const XATTR_GENERATION: &str = "user.surrealfs.generation";
pub const XATTR_MODE: &str = "user.surrealfs.mode";
pub const XATTR_BRANCH: &str = "user.surrealfs.branch";
pub const XATTR_LOCKED_BY: &str = "user.surrealfs.locked_by";
pub const XATTR_ACTION: &str = "user.surrealfs.action";
pub const XATTR_LOCK: &str = "user.surrealfs.lock";

pub const USER_AGENT_PREFIX: &str = "user.agent.";

pub struct XattrEngine;

impl XattrEngine {
    /// Lists all extended attribute names for a given file entry.
    /// Notice: Control action attributes (user.surrealfs.action, user.surrealfs.lock)
    /// are strictly write-only and MUST NEVER be returned by listxattr.
    /// This prevents `cp -a` or `rsync -X` from copying action triggers.
    pub fn list_xattrs(_entry: &FileEntry, active_lock: Option<&FileLock>) -> Vec<String> {
        let mut list = vec![
            XATTR_OWNER.to_string(),
            XATTR_GENERATION.to_string(),
            XATTR_MODE.to_string(),
            XATTR_BRANCH.to_string(),
        ];

        if active_lock.is_some() {
            list.push(XATTR_LOCKED_BY.to_string());
        }

        list
    }

    /// Gets an extended attribute value.
    pub fn get_xattr(
        entry: &FileEntry,
        name: &str,
        active_lock: Option<&FileLock>,
        custom_attrs: &HashMap<String, String>,
    ) -> Result<Option<Vec<u8>>, SurrealFsError> {
        match name {
            XATTR_OWNER => Ok(entry.owner.as_ref().map(|o| o.as_bytes().to_vec())),
            XATTR_GENERATION => Ok(Some(entry.generation.to_string().into_bytes())),
            XATTR_MODE => Ok(Some(format!("{:o}", entry.mode).into_bytes())),
            XATTR_BRANCH => Ok(Some(entry.branch.as_bytes().to_vec())),
            XATTR_LOCKED_BY => {
                if let Some(lock) = active_lock {
                    let desc = format!("{} (reason: {:?})", lock.holder, lock.reason);
                    Ok(Some(desc.into_bytes()))
                } else {
                    Ok(None)
                }
            }
            // Control action attributes are write-only
            XATTR_ACTION | XATTR_LOCK => Ok(None),
            _ => {
                if name.starts_with(USER_AGENT_PREFIX) {
                    Ok(custom_attrs.get(name).map(|v| v.as_bytes().to_vec()))
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Sets an extended attribute value or executes a virtual action.
    pub async fn set_xattr(
        fs: &SurrealFs,
        path: &str,
        name: &str,
        value: &[u8],
        caller: &str,
        custom_attrs: &mut HashMap<String, String>,
    ) -> Result<(), SurrealFsError> {
        let val_str = std::str::from_utf8(value).map_err(|e| {
            SurrealFsError::InvalidPath(format!("Invalid UTF-8 xattr value: {}", e))
        })?;

        match name {
            // Read-only system attributes reject modification with PermissionDenied (EPERM)
            XATTR_OWNER | XATTR_GENERATION | XATTR_MODE | XATTR_BRANCH | XATTR_LOCKED_BY => Err(
                SurrealFsError::PermissionDenied(format!("Attribute '{}' is read-only", name)),
            ),

            // Virtual Action Trigger: acquire advisory lock
            // Format: "lease:<ttl_seconds>:<reason>"
            XATTR_LOCK => {
                let parts: Vec<&str> = val_str.splitn(3, ':').collect();
                if parts.len() >= 2 && parts[0] == "lease" {
                    let ttl: u64 = parts[1].trim_end_matches('s').parse().unwrap_or(60);
                    let reason = if parts.len() == 3 { parts[2] } else { "" };
                    fs.acquire_lock(path, ttl, reason, caller).await?;
                    Ok(())
                } else {
                    Err(SurrealFsError::InvalidPath(
                        "Invalid lock format. Expected 'lease:<ttl>s:<reason>'".into(),
                    ))
                }
            }

            // Virtual Action Trigger: reindex embedding
            XATTR_ACTION => {
                if val_str == "reindex" {
                    Ok(())
                } else {
                    Err(SurrealFsError::InvalidPath(format!(
                        "Unknown action '{}'",
                        val_str
                    )))
                }
            }

            _ => {
                if name.starts_with(USER_AGENT_PREFIX) {
                    custom_attrs.insert(name.to_string(), val_str.to_string());
                    Ok(())
                } else {
                    Err(SurrealFsError::InvalidPath(format!(
                        "Unsupported xattr namespace '{}'",
                        name
                    )))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_entry() -> FileEntry {
        FileEntry {
            path: "/docs/readme.md".into(),
            filename: "readme.md".into(),
            is_folder: false,
            size: 1024,
            content_type: "text/markdown".into(),
            content: None,
            mode: 0o644,
            owner: Some("martin".into()),
            group: Some("engineering".into()),
            branch: "main".into(),
            generation: 3,
            parent_key: Some("root".into()),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_list_xattrs_never_includes_actions() {
        let entry = dummy_entry();
        let list = XattrEngine::list_xattrs(&entry, None);

        assert!(list.contains(&XATTR_OWNER.to_string()));
        assert!(list.contains(&XATTR_GENERATION.to_string()));
        assert!(list.contains(&XATTR_MODE.to_string()));
        assert!(list.contains(&XATTR_BRANCH.to_string()));

        // CRITICAL: action attributes must NEVER be listed
        assert!(!list.contains(&XATTR_ACTION.to_string()));
        assert!(!list.contains(&XATTR_LOCK.to_string()));
    }

    #[test]
    fn test_get_xattr_read_only_fields() {
        let entry = dummy_entry();
        let custom = HashMap::new();

        let val = XattrEngine::get_xattr(&entry, XATTR_OWNER, None, &custom)
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8(val).unwrap(), "martin");

        let gen = XattrEngine::get_xattr(&entry, XATTR_GENERATION, None, &custom)
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8(gen).unwrap(), "3");

        let act = XattrEngine::get_xattr(&entry, XATTR_ACTION, None, &custom).unwrap();
        assert!(act.is_none());
    }
}
