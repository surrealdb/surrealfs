#![allow(clippy::missing_safety_doc)]

use crate::virtual_fs::{VirtualFs, MIN_VIRTUAL_FD};
use libc::{mode_t, off_t, size_t, ssize_t, stat as libc_stat};
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::OnceLock;
use surrealfs_core::{ConnectOptions, SurrealFs};

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static VFS: OnceLock<Option<VirtualFs>> = OnceLock::new();

fn rt() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime for surrealfs-shim")
    })
}

pub fn get_vfs() -> Option<&'static VirtualFs> {
    VFS.get_or_init(|| {
        let mount = std::env::var("SURREALFS_MOUNT").unwrap_or_else(|_| "/surrealfs".into());
        let url =
            std::env::var("SURREALDB_URL").unwrap_or_else(|_| "ws://127.0.0.1:8000/rpc".into());
        let ns = std::env::var("SURREALDB_NS").unwrap_or_else(|_| "test".into());
        let db = std::env::var("SURREALDB_DB").unwrap_or_else(|_| "test".into());
        let user = std::env::var("SURREALDB_USER").unwrap_or_else(|_| "root".into());
        let pass = std::env::var("SURREALDB_PASS").unwrap_or_else(|_| "root".into());

        let opts = ConnectOptions {
            url,
            ns,
            db,
            user: Some(user),
            pass: Some(pass),
            caller: None,
        };

        match rt().block_on(SurrealFs::connect(opts)) {
            Ok(fs) => Some(VirtualFs::new(fs, &mount)),
            Err(e) => {
                eprintln!("[surrealfs-shim] Failed to connect: {}", e);
                None
            }
        }
    })
    .as_ref()
}

pub fn set_test_vfs(vfs: VirtualFs) {
    let _ = VFS.set(Some(vfs));
}

// =========================================================================
// Real libc symbol loader via dlsym(RTLD_NEXT)
// =========================================================================

macro_rules! get_real_fn {
    ($fn_name:ident, $fn_type:ty) => {{
        static REAL_FN: OnceLock<$fn_type> = OnceLock::new();
        *REAL_FN.get_or_init(|| unsafe {
            let symbol = CString::new(stringify!($fn_name)).unwrap();
            let ptr = libc::dlsym(libc::RTLD_NEXT, symbol.as_ptr());
            if ptr.is_null() {
                panic!("Failed to find real symbol {}", stringify!($fn_name));
            }
            std::mem::transmute(ptr)
        })
    }};
}

// Convert C string to Rust &str safely
fn to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok() }
}

// =========================================================================
// Intercepted POSIX symbols
// =========================================================================

#[no_mangle]
pub unsafe extern "C" fn open(pathname: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.open(path, flags, mode as u32)) {
                Ok(fd) => return fd,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_open = get_real_fn!(
        open,
        unsafe extern "C" fn(*const c_char, c_int, mode_t) -> c_int
    );
    real_open(pathname, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn openat(
    dirfd: c_int,
    pathname: *const c_char,
    flags: c_int,
    mode: mode_t,
) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.open(path, flags, mode as u32)) {
                Ok(fd) => return fd,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_openat = get_real_fn!(
        openat,
        unsafe extern "C" fn(c_int, *const c_char, c_int, mode_t) -> c_int
    );
    real_openat(dirfd, pathname, flags, mode)
}

#[no_mangle]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            match vfs.read(fd, count) {
                Ok(data) => {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, data.len());
                    return data.len() as ssize_t;
                }
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_read = get_real_fn!(
        read,
        unsafe extern "C" fn(c_int, *mut c_void, size_t) -> ssize_t
    );
    real_read(fd, buf, count)
}

#[no_mangle]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            let slice = std::slice::from_raw_parts(buf as *const u8, count);
            match vfs.write(fd, slice) {
                Ok(written) => return written as ssize_t,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_write = get_real_fn!(
        write,
        unsafe extern "C" fn(c_int, *const c_void, size_t) -> ssize_t
    );
    real_write(fd, buf, count)
}

#[no_mangle]
pub unsafe extern "C" fn pread(
    fd: c_int,
    buf: *mut c_void,
    count: size_t,
    offset: off_t,
) -> ssize_t {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            match vfs.pread(fd, count, offset as u64) {
                Ok(data) => {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), buf as *mut u8, data.len());
                    return data.len() as ssize_t;
                }
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_pread = get_real_fn!(
        pread,
        unsafe extern "C" fn(c_int, *mut c_void, size_t, off_t) -> ssize_t
    );
    real_pread(fd, buf, count, offset)
}

#[no_mangle]
pub unsafe extern "C" fn pwrite(
    fd: c_int,
    buf: *const c_void,
    count: size_t,
    offset: off_t,
) -> ssize_t {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            let slice = std::slice::from_raw_parts(buf as *const u8, count);
            match vfs.pwrite(fd, slice, offset as u64) {
                Ok(written) => return written as ssize_t,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_pwrite = get_real_fn!(
        pwrite,
        unsafe extern "C" fn(c_int, *const c_void, size_t, off_t) -> ssize_t
    );
    real_pwrite(fd, buf, count, offset)
}

#[no_mangle]
pub unsafe extern "C" fn lseek(fd: c_int, offset: off_t, whence: c_int) -> off_t {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            match vfs.lseek(fd, offset, whence) {
                Ok(new_pos) => return new_pos as off_t,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_lseek = get_real_fn!(lseek, unsafe extern "C" fn(c_int, off_t, c_int) -> off_t);
    real_lseek(fd, offset, whence)
}

#[no_mangle]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            match rt().block_on(vfs.close(fd)) {
                Ok(()) => return 0,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_close = get_real_fn!(close, unsafe extern "C" fn(c_int) -> c_int);
    real_close(fd)
}

#[no_mangle]
pub unsafe extern "C" fn stat(pathname: *const c_char, statbuf: *mut libc_stat) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.stat(path)) {
                Ok(entry) => {
                    fill_stat(
                        statbuf,
                        entry.is_folder,
                        entry.size as off_t,
                        entry.mode as mode_t,
                    );
                    return 0;
                }
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_stat = get_real_fn!(
        stat,
        unsafe extern "C" fn(*const c_char, *mut libc_stat) -> c_int
    );
    real_stat(pathname, statbuf)
}

#[no_mangle]
pub unsafe extern "C" fn lstat(pathname: *const c_char, statbuf: *mut libc_stat) -> c_int {
    stat(pathname, statbuf)
}

#[no_mangle]
pub unsafe extern "C" fn fstat(fd: c_int, statbuf: *mut libc_stat) -> c_int {
    if fd >= MIN_VIRTUAL_FD {
        if let Some(vfs) = get_vfs() {
            match vfs.fstat(fd) {
                Ok(file) => {
                    fill_stat(statbuf, false, file.buffer.len() as off_t, 0o644);
                    return 0;
                }
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_fstat = get_real_fn!(fstat, unsafe extern "C" fn(c_int, *mut libc_stat) -> c_int);
    real_fstat(fd, statbuf)
}

#[no_mangle]
pub unsafe extern "C" fn mkdir(pathname: *const c_char, mode: mode_t) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.mkdir(path)) {
                Ok(()) => return 0,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_mkdir = get_real_fn!(mkdir, unsafe extern "C" fn(*const c_char, mode_t) -> c_int);
    real_mkdir(pathname, mode)
}

#[no_mangle]
pub unsafe extern "C" fn unlink(pathname: *const c_char) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.unlink(path)) {
                Ok(()) => return 0,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_unlink = get_real_fn!(unlink, unsafe extern "C" fn(*const c_char) -> c_int);
    real_unlink(pathname)
}

#[no_mangle]
pub unsafe extern "C" fn rmdir(pathname: *const c_char) -> c_int {
    if let (Some(path), Some(vfs)) = (to_str(pathname), get_vfs()) {
        if vfs.is_virtual_path(path) {
            match rt().block_on(vfs.rmdir(path)) {
                Ok(()) => return 0,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_rmdir = get_real_fn!(rmdir, unsafe extern "C" fn(*const c_char) -> c_int);
    real_rmdir(pathname)
}

#[no_mangle]
pub unsafe extern "C" fn rename(oldpath: *const c_char, newpath: *const c_char) -> c_int {
    if let (Some(src), Some(dst), Some(vfs)) = (to_str(oldpath), to_str(newpath), get_vfs()) {
        if vfs.is_virtual_path(src) && vfs.is_virtual_path(dst) {
            match rt().block_on(vfs.rename(src, dst)) {
                Ok(()) => return 0,
                Err(e) => {
                    errno::set_errno(errno_from_err(&e));
                    return -1;
                }
            }
        }
    }

    let real_rename = get_real_fn!(
        rename,
        unsafe extern "C" fn(*const c_char, *const c_char) -> c_int
    );
    real_rename(oldpath, newpath)
}

unsafe fn fill_stat(statbuf: *mut libc_stat, is_folder: bool, size: off_t, mode_bits: mode_t) {
    if statbuf.is_null() {
        return;
    }
    std::ptr::write_bytes(statbuf, 0, 1);
    let s = &mut *statbuf;
    s.st_mode = if is_folder {
        libc::S_IFDIR | if mode_bits == 0 { 0o755 } else { mode_bits }
    } else {
        libc::S_IFREG | if mode_bits == 0 { 0o644 } else { mode_bits }
    };
    s.st_size = size;
    s.st_nlink = if is_folder { 2 } else { 1 };
}

mod errno {
    pub fn set_errno(err: libc::c_int) {
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = err;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = err;
        }
    }
}

fn errno_from_err(err: &surrealfs_core::SurrealFsError) -> libc::c_int {
    match err {
        surrealfs_core::SurrealFsError::NotFound(_) => libc::ENOENT,
        surrealfs_core::SurrealFsError::PermissionDenied(_) => libc::EACCES,
        surrealfs_core::SurrealFsError::AlreadyExists(_) => libc::EEXIST,
        surrealfs_core::SurrealFsError::DirectoryNotEmpty(_) => libc::ENOTEMPTY,
        _ => libc::EIO,
    }
}
