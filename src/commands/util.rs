use surrealfs::FsError;

pub fn resolve_cli_path(cwd: &str, input: &str) -> String {
    if input.starts_with('/') {
        input.to_string()
    } else {
        let mut combined = cwd.to_string();
        if !combined.ends_with('/') {
            combined.push('/');
        }
        combined.push_str(input);
        combined
    }
}

pub fn help_error() -> FsError {
    FsError::InvalidPath
}
