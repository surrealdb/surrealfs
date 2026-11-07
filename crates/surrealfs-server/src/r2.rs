//! A [`BlobStore`] over a Cloudflare R2 bucket binding.
//!
//! Every call goes through the binding's JavaScript API, so objects and listings are moved by the
//! runtime and not through the database. A chunk is at most a few MiB, so `get` reads the whole
//! object and slices a requested range in memory.

use std::ops::Range;

use async_trait::async_trait;
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use surrealfs_core::{BlobInfo, BlobStore, Result, SurrealFsError};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

fn storage(context: &str, e: JsValue) -> SurrealFsError {
    SurrealFsError::Storage(format!("{context}: {}", describe(&e)))
}

fn describe(e: &JsValue) -> String {
    e.as_string()
        .or_else(|| {
            Reflect::get(e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| format!("{e:?}"))
}

/// Call `object.method(...args)` and await the result when it is a promise.
pub(crate) async fn call(object: &JsValue, method: &str, args: &[JsValue]) -> Result<JsValue> {
    let function: Function = Reflect::get(object, &method.into())
        .map_err(|e| storage(method, e))?
        .dyn_into()
        .map_err(|_| SurrealFsError::Storage(format!("{method} is not a function")))?;
    let list = Array::new();
    for arg in args {
        list.push(arg);
    }
    let result = function
        .apply(object, &list)
        .map_err(|e| storage(method, e))?;
    match result.dyn_into::<Promise>() {
        Ok(promise) => JsFuture::from(promise)
            .await
            .map_err(|e| storage(method, e)),
        Err(plain) => Ok(plain),
    }
}

pub struct R2BlobStore {
    bucket: JsValue,
}

// A Worker isolate runs one thread, and the binding is only ever used from it.
unsafe impl Send for R2BlobStore {}
unsafe impl Sync for R2BlobStore {}

impl R2BlobStore {
    pub fn new(bucket: JsValue) -> Self {
        Self { bucket }
    }
}

fn info(object: &JsValue) -> Option<BlobInfo> {
    let key = Reflect::get(object, &"key".into()).ok()?.as_string()?;
    let size = Reflect::get(object, &"size".into()).ok()?.as_f64()? as u64;
    let modified = Reflect::get(object, &"uploaded".into())
        .ok()
        .and_then(|d| Reflect::get(&d, &"getTime".into()).ok().map(|f| (d, f)))
        .and_then(|(d, f)| f.dyn_into::<Function>().ok()?.call0(&d).ok())
        .and_then(|t| t.as_f64())
        .map(|ms| (ms / 1000.0) as i64);
    Some(BlobInfo {
        key,
        size,
        modified,
    })
}

#[async_trait(?Send)]
impl BlobStore for R2BlobStore {
    async fn put(&self, key: &str, data: Vec<u8>) -> Result<()> {
        let body = Uint8Array::from(data.as_slice());
        call(&self.bucket, "put", &[key.into(), body.into()]).await?;
        Ok(())
    }

    async fn get(&self, key: &str, range: Option<Range<u64>>) -> Result<Vec<u8>> {
        let object = call(&self.bucket, "get", &[key.into()]).await?;
        if object.is_null() || object.is_undefined() {
            return Err(SurrealFsError::NotFound(format!("blob {key}")));
        }
        let buffer = call(&object, "arrayBuffer", &[]).await?;
        let bytes = Uint8Array::new(&buffer).to_vec();
        Ok(match range {
            None => bytes,
            Some(r) => {
                let start = (r.start as usize).min(bytes.len());
                let end = (r.end as usize).min(bytes.len()).max(start);
                bytes[start..end].to_vec()
            }
        })
    }

    async fn head(&self, key: &str) -> Result<Option<BlobInfo>> {
        let object = call(&self.bucket, "head", &[key.into()]).await?;
        if object.is_null() || object.is_undefined() {
            return Ok(None);
        }
        Ok(info(&object))
    }

    async fn delete(&self, key: &str) -> Result<()> {
        call(&self.bucket, "delete", &[key.into()]).await?;
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<BlobInfo>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let options = Object::new();
            let set = |k: &str, v: JsValue| Reflect::set(&options, &k.into(), &v);
            let _ = set("prefix", prefix.into());
            let _ = set("limit", 1000.into());
            if let Some(c) = &cursor {
                let _ = set("cursor", c.as_str().into());
            }
            let page = call(&self.bucket, "list", &[options.into()]).await?;
            let objects = Reflect::get(&page, &"objects".into()).map_err(|e| storage("list", e))?;
            for object in Array::from(&objects).iter() {
                if let Some(i) = info(&object) {
                    out.push(i);
                }
            }
            let truncated = Reflect::get(&page, &"truncated".into())
                .ok()
                .and_then(|t| t.as_bool())
                .unwrap_or(false);
            cursor = Reflect::get(&page, &"cursor".into())
                .ok()
                .and_then(|c| c.as_string());
            if !truncated || cursor.is_none() {
                return Ok(out);
            }
        }
    }
}
