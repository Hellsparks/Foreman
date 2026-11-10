//! Moonraker file API (`/server/files/*`) for the `gcodes` root, as [`PrinterStorage`].
//!
//! Verified against moonraker.readthedocs.io (external_api/file_manager):
//! `GET /server/files/directory?path=gcodes/<dir>&extended=true`, multipart
//! `POST /server/files/upload` (`root`, `path`, `file`), `GET|DELETE /server/files/gcodes/<file>`,
//! `POST|DELETE /server/files/directory`, `POST /server/files/move|copy {source, dest}`.
//! HTTP results are wrapped as `{"result": …}`; errors as `{"error": {"message": …}}`.

use std::{
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt as _, TryStreamExt as _};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt as _;

use crate::{
    error::AppError,
    storage::{
        clean_path, join, name_of, parent_of, ByteStream, PrinterStorage, RemoteFileMeta,
        StorageEntry,
    },
};

const ROOT: &str = "gcodes";

pub struct MoonrakerFiles {
    base: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

fn http_err(e: reqwest::Error) -> AppError {
    AppError::Driver(format!("moonraker http: {e}"))
}

async fn check(resp: reqwest::Response) -> Result<reqwest::Response, AppError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let msg = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| status.canonical_reason().unwrap_or("error"))
        .to_owned();
    Err(match status.as_u16() {
        404 => AppError::NotFound(msg),
        400 | 403 | 409 | 422 => AppError::InvalidInput(msg),
        _ => AppError::Driver(format!("moonraker {status}: {msg}")),
    })
}

async fn result(resp: reqwest::Response) -> Result<Value, AppError> {
    let mut v: Value = check(resp).await?.json().await.map_err(http_err)?;
    Ok(v.get_mut("result").map(Value::take).unwrap_or(v))
}

fn rooted(path: &str) -> Result<String, AppError> {
    let p = clean_path(path)?;
    Ok(if p.is_empty() {
        ROOT.to_owned()
    } else {
        format!("{ROOT}/{p}")
    })
}

fn meta_of(file: &Value, folder: &str) -> Option<RemoteFileMeta> {
    let s = |k: &str| file.get(k).and_then(Value::as_str).map(str::to_owned);
    let f = |k: &str| file.get(k).and_then(Value::as_f64);
    let thumb = file
        .get("thumbnails")
        .and_then(Value::as_array)
        .and_then(|t| {
            t.iter().max_by_key(|x| {
                x.get("width").and_then(Value::as_u64).unwrap_or(0)
                    * x.get("height").and_then(Value::as_u64).unwrap_or(0)
            })
        })
        .and_then(|t| t.get("relative_path").and_then(Value::as_str))
        .and_then(|rel| clean_path(&join(folder, rel)).ok());
    let m = RemoteFileMeta {
        slicer: s("slicer"),
        slicer_version: s("slicer_version"),
        estimated_time: f("estimated_time"),
        filament_type: s("filament_type"),
        filament_name: s("filament_name"),
        filament_weight_total: f("filament_weight_total"),
        nozzle_diameter: f("nozzle_diameter"),
        layer_height: f("layer_height"),
        filament_colors: file
            .get("filament_colors")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        thumb_path: thumb,
    };
    (m.slicer.is_some() || m.estimated_time.is_some() || m.thumb_path.is_some()).then_some(m)
}

impl MoonrakerFiles {
    pub fn new(base_url: &str, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();
        Self {
            base: base_url.trim_end_matches('/').to_owned(),
            api_key,
            http,
        }
    }

    fn req(&self, method: reqwest::Method, url: reqwest::Url) -> reqwest::RequestBuilder {
        let r = self.http.request(method, url);
        match &self.api_key {
            Some(k) if !k.is_empty() => r.header("X-Api-Key", k),
            _ => r,
        }
    }

    fn url(&self, endpoint: &str) -> Result<reqwest::Url, AppError> {
        reqwest::Url::parse(&format!("{}{endpoint}", self.base))
            .map_err(|e| AppError::Driver(format!("bad moonraker url: {e}")))
    }

    /// `/server/files/gcodes/<segments…>` with each segment percent-encoded.
    fn file_url(&self, path: &str) -> Result<reqwest::Url, AppError> {
        let p = clean_path(path)?;
        if p.is_empty() {
            return Err(AppError::InvalidInput("no file path".into()));
        }
        let mut url = self.url("/server/files")?;
        url.path_segments_mut()
            .map_err(|_| AppError::Driver("bad moonraker url".into()))?
            .push(ROOT)
            .extend(p.split('/'));
        Ok(url)
    }

    async fn post_json(&self, endpoint: &str, body: Value) -> Result<Value, AppError> {
        let resp = self
            .req(reqwest::Method::POST, self.url(endpoint)?)
            .timeout(Duration::from_secs(60))
            .json(&body)
            .send()
            .await
            .map_err(http_err)?;
        result(resp).await
    }

    async fn upload_part(
        &self,
        dest: &str,
        part: reqwest::multipart::Part,
    ) -> Result<(), AppError> {
        let dest = clean_path(dest)?;
        let name = name_of(&dest).to_owned();
        if name.is_empty() {
            return Err(AppError::InvalidInput("no file name".into()));
        }
        let mut form = reqwest::multipart::Form::new().text("root", ROOT);
        let folder = parent_of(&dest);
        if !folder.is_empty() {
            form = form.text("path", folder.to_owned());
        }
        let form = form.part("file", part.file_name(name));
        let resp = self
            .req(reqwest::Method::POST, self.url("/server/files/upload")?)
            .multipart(form)
            .send()
            .await
            .map_err(http_err)?;
        check(resp).await?;
        Ok(())
    }

    /// Upload an in-memory file (driver trait's `upload_file`).
    pub async fn upload_bytes(&self, dest: &str, data: Vec<u8>) -> Result<(), AppError> {
        self.upload_part(dest, reqwest::multipart::Part::bytes(data))
            .await
    }
}

#[async_trait]
impl PrinterStorage for MoonrakerFiles {
    async fn list_dir(&self, path: &str) -> Result<Vec<StorageEntry>, AppError> {
        let folder = clean_path(path)?;
        let resp = self
            .req(reqwest::Method::GET, self.url("/server/files/directory")?)
            .timeout(Duration::from_secs(30))
            .query(&[("path", rooted(&folder)?.as_str()), ("extended", "true")])
            .send()
            .await
            .map_err(http_err)?;
        let r = result(resp).await?;
        let mut out = Vec::new();
        let list = |k: &str| {
            r.get(k)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        for d in list("dirs") {
            let Some(name) = d.get("dirname").and_then(Value::as_str) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            out.push(StorageEntry {
                name: name.to_owned(),
                path: join(&folder, name),
                is_dir: true,
                size: 0,
                modified: d.get("modified").and_then(Value::as_f64).unwrap_or(0.0),
                meta: None,
            });
        }
        for f in list("files") {
            let Some(name) = f.get("filename").and_then(Value::as_str) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            out.push(StorageEntry {
                name: name.to_owned(),
                path: join(&folder, name),
                is_dir: false,
                size: f.get("size").and_then(Value::as_u64).unwrap_or(0),
                modified: f.get("modified").and_then(Value::as_f64).unwrap_or(0.0),
                meta: meta_of(&f, &folder),
            });
        }
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    async fn upload(
        &self,
        dest: &str,
        src: &Path,
        progress: Arc<AtomicU64>,
    ) -> Result<(), AppError> {
        let file = tokio::fs::File::open(src)
            .await
            .map_err(|e| AppError::Internal(format!("open {}: {e}", src.display())))?;
        let len = file
            .metadata()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
            .len();
        let stream =
            tokio_util::io::ReaderStream::with_capacity(file, 256 << 10).inspect_ok(move |chunk| {
                progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
            });
        let body = reqwest::Body::wrap_stream(stream);
        self.upload_part(
            dest,
            reqwest::multipart::Part::stream_with_length(body, len),
        )
        .await
    }

    async fn download(
        &self,
        path: &str,
        dest: &Path,
        progress: Arc<AtomicU64>,
    ) -> Result<u64, AppError> {
        let resp = self
            .req(reqwest::Method::GET, self.file_url(path)?)
            .send()
            .await
            .map_err(http_err)?;
        let mut resp = check(resp).await?;
        let mut out = tokio::fs::File::create(dest)
            .await
            .map_err(|e| AppError::Internal(format!("create {}: {e}", dest.display())))?;
        let mut n = 0u64;
        while let Some(chunk) = resp.chunk().await.map_err(http_err)? {
            out.write_all(&chunk)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            n += chunk.len() as u64;
            progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        }
        out.flush()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(n)
    }

    async fn read_stream(&self, path: &str) -> Result<(Option<u64>, ByteStream), AppError> {
        let resp = self
            .req(reqwest::Method::GET, self.file_url(path)?)
            .send()
            .await
            .map_err(http_err)?;
        let resp = check(resp).await?;
        let len = resp.content_length();
        let stream = resp
            .bytes_stream()
            .map(|r| r.map(|b| b.to_vec()).map_err(http_err))
            .boxed();
        Ok((len, stream))
    }

    async fn delete_file(&self, path: &str) -> Result<(), AppError> {
        let resp = self
            .req(reqwest::Method::DELETE, self.file_url(path)?)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(http_err)?;
        check(resp).await?;
        Ok(())
    }

    async fn delete_dir(&self, path: &str) -> Result<(), AppError> {
        let p = clean_path(path)?;
        if p.is_empty() {
            return Err(AppError::InvalidInput(
                "refusing to delete the gcodes root".into(),
            ));
        }
        let resp = self
            .req(
                reqwest::Method::DELETE,
                self.url("/server/files/directory")?,
            )
            .timeout(Duration::from_secs(60))
            .query(&[("path", rooted(&p)?.as_str()), ("force", "true")])
            .send()
            .await
            .map_err(http_err)?;
        check(resp).await?;
        Ok(())
    }

    async fn mkdir(&self, path: &str) -> Result<(), AppError> {
        self.post_json("/server/files/directory", json!({ "path": rooted(path)? }))
            .await?;
        Ok(())
    }

    async fn move_entry(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.post_json(
            "/server/files/move",
            json!({ "source": rooted(from)?, "dest": rooted(to)? }),
        )
        .await?;
        Ok(())
    }

    async fn copy_entry(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.post_json(
            "/server/files/copy",
            json!({ "source": rooted(from)?, "dest": rooted(to)? }),
        )
        .await?;
        Ok(())
    }

    async fn active_file(&self) -> Result<Option<String>, AppError> {
        let resp = self
            .req(reqwest::Method::GET, self.url("/printer/objects/query")?)
            .timeout(Duration::from_secs(10))
            .query(&[("print_stats", "state,filename")])
            .send()
            .await
            .map_err(http_err)?;
        let r = result(resp).await?;
        let ps = r
            .pointer("/status/print_stats")
            .cloned()
            .unwrap_or(Value::Null);
        let state = ps.get("state").and_then(Value::as_str).unwrap_or("");
        let file = ps.get("filename").and_then(Value::as_str).unwrap_or("");
        Ok((matches!(state, "printing" | "paused") && !file.is_empty())
            .then(|| clean_path(file).unwrap_or_else(|_| file.to_owned())))
    }
}
