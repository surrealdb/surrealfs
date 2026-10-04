use surrealfs_core::SurrealFsError;

pub const ENOENT: i32 = 2;
pub const EIO: i32 = 5;
pub const EACCES: i32 = 13;
pub const EEXIST: i32 = 17;
pub const EISDIR: i32 = 21;
pub const EINVAL: i32 = 22;
pub const ENOTEMPTY: i32 = 66;
pub const ESTALE: i32 = 70;

pub fn error_to_errno(err: &SurrealFsError) -> i32 {
    match err {
        SurrealFsError::NotFound(_) => ENOENT,
        SurrealFsError::PermissionDenied(_) => EACCES,
        SurrealFsError::Conflict(_) => ESTALE,
        SurrealFsError::AlreadyExists(_) => EEXIST,
        SurrealFsError::IsADirectory(_) => EISDIR,
        SurrealFsError::DirectoryNotEmpty(_) => ENOTEMPTY,
        SurrealFsError::InvalidPath(_) => EINVAL,
        _ => EIO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_errno_mapping() {
        assert_eq!(
            error_to_errno(&SurrealFsError::NotFound("lost".into())),
            ENOENT
        );
        assert_eq!(
            error_to_errno(&SurrealFsError::PermissionDenied("no".into())),
            EACCES
        );
        assert_eq!(
            error_to_errno(&SurrealFsError::Conflict("gen".into())),
            ESTALE
        );
        assert_eq!(
            error_to_errno(&SurrealFsError::AlreadyExists("dup".into())),
            EEXIST
        );
        assert_eq!(
            error_to_errno(&SurrealFsError::IsADirectory("dir".into())),
            EISDIR
        );
        assert_eq!(
            error_to_errno(&SurrealFsError::DirectoryNotEmpty("full".into())),
            ENOTEMPTY
        );
    }
}
