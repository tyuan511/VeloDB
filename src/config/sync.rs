use crate::adapter::ConnectionConfig;
use crate::config::{config_dir, AppSettings, SqlSnippet};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE, HOST};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::time::Duration;
use url::Url;

type HmacSha256 = Hmac<Sha256>;

pub const SYNC_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncProvider {
    S3,
    WebDav,
}

impl Default for SyncProvider {
    fn default() -> Self {
        Self::S3
    }
}

impl SyncProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::S3 => "S3 兼容存储",
            Self::WebDav => "WebDAV",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncPullStrategy {
    Merge,
    Overwrite,
}

impl Default for SyncPullStrategy {
    fn default() -> Self {
        Self::Merge
    }
}

impl SyncPullStrategy {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Merge => "合并主机（保留本地独有）",
            Self::Overwrite => "完全覆盖本地",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3SyncConfig {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub session_token: String,
    pub object_key: String,
    pub path_style: bool,
}

impl Default for S3SyncConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://s3.amazonaws.com".to_string(),
            region: "us-east-1".to_string(),
            bucket: String::new(),
            access_key_id: String::new(),
            secret_access_key: String::new(),
            session_token: String::new(),
            object_key: "velodb/config-sync.json".to_string(),
            path_style: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavSyncConfig {
    pub endpoint: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
}

impl Default for WebDavSyncConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            username: String::new(),
            password: String::new(),
            remote_path: "velodb/config-sync.json".to_string(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_device_id() -> String {
    format!("dev-{}", uuid::Uuid::new_v4().simple())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: SyncProvider,
    #[serde(default)]
    pub pull_strategy: SyncPullStrategy,
    #[serde(default)]
    pub auto_sync_on_startup: bool,
    #[serde(default)]
    pub auto_push_on_change: bool,
    #[serde(default = "default_true")]
    pub sync_snippets: bool,
    #[serde(default = "default_true")]
    pub sync_favorites: bool,
    #[serde(default = "default_device_id")]
    pub device_id: String,
    #[serde(default)]
    pub last_sync_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_sync_status: Option<String>,
    #[serde(default)]
    pub s3: S3SyncConfig,
    #[serde(default)]
    pub webdav: WebDavSyncConfig,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: SyncProvider::S3,
            pull_strategy: SyncPullStrategy::Merge,
            auto_sync_on_startup: false,
            auto_push_on_change: false,
            sync_snippets: true,
            sync_favorites: true,
            device_id: default_device_id(),
            last_sync_at: None,
            last_sync_status: None,
            s3: S3SyncConfig::default(),
            webdav: WebDavSyncConfig::default(),
        }
    }
}

impl SyncConfig {
    pub fn is_configured(&self) -> bool {
        match self.provider {
            SyncProvider::S3 => {
                !self.s3.endpoint.trim().is_empty()
                    && !self.s3.bucket.trim().is_empty()
                    && !self.s3.access_key_id.trim().is_empty()
                    && !self.s3.secret_access_key.trim().is_empty()
            }
            SyncProvider::WebDav => !self.webdav.endpoint.trim().is_empty(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSnapshot {
    pub schema_version: u32,
    pub updated_at: DateTime<Utc>,
    pub device_id: String,
    pub device_name: String,
    pub settings: AppSettings,
    pub connections: Vec<ConnectionConfig>,
    #[serde(default)]
    pub snippets: Option<Vec<SqlSnippet>>,
    #[serde(default)]
    pub favorites: Option<Vec<String>>,
}

impl SyncSnapshot {
    pub fn build(
        device_id: String,
        settings: AppSettings,
        connections: Vec<ConnectionConfig>,
        snippets: Option<Vec<SqlSnippet>>,
        favorites: Option<&HashSet<String>>,
    ) -> Self {
        let fav_vec = favorites.map(|set| {
            let mut v: Vec<String> = set.iter().cloned().collect();
            v.sort();
            v
        });
        Self {
            schema_version: SYNC_SCHEMA_VERSION,
            updated_at: Utc::now(),
            device_id,
            device_name: current_device_name(),
            settings,
            connections,
            snippets,
            favorites: fav_vec,
        }
    }
}

pub fn current_device_name() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "VeloDB Desktop".to_string())
}

pub fn load_sync_config() -> SyncConfig {
    let path = config_dir().join("sync.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_sync_config(cfg: &SyncConfig) {
    let path = config_dir().join("sync.json");
    let _ = fs::write(
        path,
        serde_json::to_string_pretty(cfg).unwrap_or_default(),
    );
}

pub fn merge_connections(
    local: &[ConnectionConfig],
    remote: &[ConnectionConfig],
    strategy: SyncPullStrategy,
) -> Vec<ConnectionConfig> {
    match strategy {
        SyncPullStrategy::Overwrite => remote.to_vec(),
        SyncPullStrategy::Merge => {
            let remote_ids: HashSet<&str> = remote.iter().map(|c| c.id.as_str()).collect();
            let mut merged = remote.to_vec();
            for conn in local {
                if !remote_ids.contains(conn.id.as_str()) {
                    merged.push(conn.clone());
                }
            }
            merged
        }
    }
}

pub fn merge_snippets(
    local: &[SqlSnippet],
    remote: &[SqlSnippet],
    strategy: SyncPullStrategy,
) -> Vec<SqlSnippet> {
    match strategy {
        SyncPullStrategy::Overwrite => remote.to_vec(),
        SyncPullStrategy::Merge => {
            let remote_ids: HashSet<&str> = remote.iter().map(|s| s.id.as_str()).collect();
            let mut merged = remote.to_vec();
            for snip in local {
                if !remote_ids.contains(snip.id.as_str()) {
                    merged.push(snip.clone());
                }
            }
            merged
        }
    }
}

pub fn merge_favorites(
    local: &HashSet<String>,
    remote: &[String],
    strategy: SyncPullStrategy,
) -> HashSet<String> {
    match strategy {
        SyncPullStrategy::Overwrite => remote.iter().cloned().collect(),
        SyncPullStrategy::Merge => {
            let mut merged = local.clone();
            merged.extend(remote.iter().cloned());
            merged
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncTestResult {
    pub message: String,
    pub remote_exists: bool,
}

fn build_http_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(format!("VeloDB/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .context("Failed to initialize HTTP client")
}

pub async fn test_sync_connection(cfg: &SyncConfig) -> Result<SyncTestResult> {
    match cfg.provider {
        SyncProvider::S3 => s3_test_connection(&cfg.s3).await,
        SyncProvider::WebDav => webdav_test_connection(&cfg.webdav).await,
    }
}

pub async fn push_snapshot(cfg: &SyncConfig, snapshot: &SyncSnapshot) -> Result<()> {
    let payload = serde_json::to_vec_pretty(snapshot).context("Failed to serialize sync snapshot")?;
    match cfg.provider {
        SyncProvider::S3 => s3_put_object(&cfg.s3, payload).await,
        SyncProvider::WebDav => webdav_put_file(&cfg.webdav, payload).await,
    }
}

pub async fn pull_snapshot(cfg: &SyncConfig) -> Result<Option<SyncSnapshot>> {
    let raw = match cfg.provider {
        SyncProvider::S3 => s3_get_object(&cfg.s3).await?,
        SyncProvider::WebDav => webdav_get_file(&cfg.webdav).await?,
    };
    match raw {
        None => Ok(None),
        Some(bytes) => {
            let snap: SyncSnapshot = serde_json::from_slice(&bytes)
                .context("远端配置文件格式无效（无法解析 JSON 快照）")?;
            Ok(Some(snap))
        }
    }
}

// ============================================================================
// S3 Compatible Storage (AWS Signature Version 4)
// ============================================================================

fn normalize_object_key(key: &str) -> String {
    let trimmed = key.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        "velodb/config-sync.json".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn build_s3_object_url(cfg: &S3SyncConfig) -> Result<(Url, String)> {
    let endpoint_str = cfg.endpoint.trim();
    if endpoint_str.is_empty() {
        bail!("S3 Endpoint 不能为空");
    }
    let bucket = cfg.bucket.trim();
    if bucket.is_empty() {
        bail!("S3 Bucket 不能为空");
    }

    let endpoint_with_scheme = if endpoint_str.starts_with("http://") || endpoint_str.starts_with("https://") {
        endpoint_str.to_string()
    } else {
        format!("https://{endpoint_str}")
    };

    let mut url = Url::parse(&endpoint_with_scheme)
        .with_context(|| format!("无效的 S3 Endpoint URL: {endpoint_str}"))?;

    let key = normalize_object_key(&cfg.object_key);
    let encoded_key = encode_s3_path(&key);

    if cfg.path_style {
        let base_path = url.path().trim_end_matches('/');
        let encoded_bucket = encode_uri_segment(bucket);
        let full_path = if base_path.is_empty() {
            format!("/{encoded_bucket}/{encoded_key}")
        } else {
            format!("{base_path}/{encoded_bucket}/{encoded_key}")
        };
        url.set_path(&full_path);
    } else {
        let host = url
            .host_str()
            .ok_or_else(|| anyhow!("S3 Endpoint 缺少主机名"))?
            .to_string();
        let new_host = format!("{bucket}.{host}");
        url.set_host(Some(&new_host))
            .map_err(|e| anyhow!("构造 Virtual-Hosted S3 域名失败: {e}"))?;
        let base_path = url.path().trim_end_matches('/');
        let full_path = if base_path.is_empty() {
            format!("/{encoded_key}")
        } else {
            format!("{base_path}/{encoded_key}")
        };
        url.set_path(&full_path);
    }

    let canonical_uri = url.path().to_string();
    Ok((url, canonical_uri))
}

fn encode_uri_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn encode_s3_path(path: &str) -> String {
    path.split('/')
        .map(encode_uri_segment)
        .collect::<Vec<_>>()
        .join("/")
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn host_header_for_url(url: &Url) -> Result<String> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URL 缺少主机名"))?;
    match url.port() {
        Some(port) => Ok(format!("{host}:{port}")),
        None => Ok(host.to_string()),
    }
}

pub(crate) fn sign_s3_headers(
    method: &Method,
    url: &Url,
    canonical_uri: &str,
    payload: &[u8],
    cfg: &S3SyncConfig,
    now: DateTime<Utc>,
) -> Result<HeaderMap> {
    let access_key = cfg.access_key_id.trim();
    let secret_key = cfg.secret_access_key.trim();
    if access_key.is_empty() || secret_key.is_empty() {
        bail!("S3 Access Key ID 和 Secret Access Key 不能为空");
    }
    let region = if cfg.region.trim().is_empty() {
        "us-east-1"
    } else {
        cfg.region.trim()
    };

    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let payload_hash = sha256_hex(payload);
    let host_header = host_header_for_url(url)?;

    let mut canonical_headers_list: Vec<(&str, String)> = vec![
        ("host", host_header.clone()),
        ("x-amz-content-sha256", payload_hash.clone()),
        ("x-amz-date", amz_date.clone()),
    ];
    let session_token = cfg.session_token.trim();
    if !session_token.is_empty() {
        canonical_headers_list.push(("x-amz-security-token", session_token.to_string()));
    }
    canonical_headers_list.sort_by(|a, b| a.0.cmp(b.0));

    let mut canonical_headers = String::new();
    let mut signed_headers_names = Vec::with_capacity(canonical_headers_list.len());
    for (k, v) in &canonical_headers_list {
        canonical_headers.push_str(k);
        canonical_headers.push(':');
        canonical_headers.push_str(v.trim());
        canonical_headers.push('\n');
        signed_headers_names.push(*k);
    }
    let signed_headers = signed_headers_names.join(";");

    let canonical_query = url.query().unwrap_or("");
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.as_str(),
        canonical_uri,
        canonical_query,
        canonical_headers,
        signed_headers,
        payload_hash
    );

    let credential_scope = format!("{date_stamp}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        amz_date,
        credential_scope,
        sha256_hex(canonical_request.as_bytes())
    );

    let k_date = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, b"s3");
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex::encode(hmac_sha256(&k_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    let mut headers = HeaderMap::new();
    headers.insert(HOST, HeaderValue::from_str(&host_header)?);
    headers.insert(
        HeaderName::from_static("x-amz-content-sha256"),
        HeaderValue::from_str(&payload_hash)?,
    );
    headers.insert(
        HeaderName::from_static("x-amz-date"),
        HeaderValue::from_str(&amz_date)?,
    );
    if !session_token.is_empty() {
        headers.insert(
            HeaderName::from_static("x-amz-security-token"),
            HeaderValue::from_str(session_token)?,
        );
    }
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&authorization)?);
    Ok(headers)
}

async fn s3_test_connection(cfg: &S3SyncConfig) -> Result<SyncTestResult> {
    let (url, canonical_uri) = build_s3_object_url(cfg)?;
    let headers = sign_s3_headers(&Method::HEAD, &url, &canonical_uri, &[], cfg, Utc::now())?;
    let client = build_http_client()?;
    let resp = client
        .head(url)
        .headers(headers)
        .send()
        .await
        .context("无法连接到 S3 服务器")?;

    match resp.status() {
        s if s.is_success() => Ok(SyncTestResult {
            message: "S3 连接成功（远端已存在同步配置）".to_string(),
            remote_exists: true,
        }),
        StatusCode::NOT_FOUND => Ok(SyncTestResult {
            message: "S3 连接成功（远端尚无配置，可点击上传）".to_string(),
            remote_exists: false,
        }),
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => {
            bail!("S3 鉴权失败 (HTTP {})，请检查 Access Key / Secret Key / Region 与权限", resp.status().as_u16())
        }
        other => {
            bail!("S3 响应异常 (HTTP {})", other.as_u16())
        }
    }
}

async fn s3_put_object(cfg: &S3SyncConfig, payload: Vec<u8>) -> Result<()> {
    let (url, canonical_uri) = build_s3_object_url(cfg)?;
    let mut headers = sign_s3_headers(&Method::PUT, &url, &canonical_uri, &payload, cfg, Utc::now())?;
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );

    let client = build_http_client()?;
    let resp = client
        .put(url)
        .headers(headers)
        .body(payload)
        .send()
        .await
        .context("上传配置到 S3 失败")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        bail!("S3 上传失败 (HTTP {}): {}", status.as_u16(), snippet);
    }
    Ok(())
}

async fn s3_get_object(cfg: &S3SyncConfig) -> Result<Option<Vec<u8>>> {
    let (url, canonical_uri) = build_s3_object_url(cfg)?;
    let headers = sign_s3_headers(&Method::GET, &url, &canonical_uri, &[], cfg, Utc::now())?;
    let client = build_http_client()?;
    let resp = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .context("从 S3 拉取配置失败")?;

    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        bail!("S3 拉取失败 (HTTP {}): {}", status.as_u16(), snippet);
    }
    let bytes = resp.bytes().await.context("读取 S3 响应内容失败")?;
    Ok(Some(bytes.to_vec()))
}

// ============================================================================
// WebDAV Storage
// ============================================================================

fn normalize_webdav_path(path: &str) -> String {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        "velodb/config-sync.json".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn build_webdav_urls(cfg: &WebDavSyncConfig) -> Result<(Url, Vec<Url>, Url)> {
    let endpoint_str = cfg.endpoint.trim();
    if endpoint_str.is_empty() {
        bail!("WebDAV 服务器地址不能为空");
    }
    let endpoint_with_scheme = if endpoint_str.starts_with("http://") || endpoint_str.starts_with("https://") {
        endpoint_str.to_string()
    } else {
        format!("https://{endpoint_str}")
    };
    let mut base_url = Url::parse(&endpoint_with_scheme)
        .with_context(|| format!("无效的 WebDAV 地址: {endpoint_str}"))?;
    if !base_url.path().ends_with('/') {
        let p = format!("{}/", base_url.path());
        base_url.set_path(&p);
    }

    let rel_path = normalize_webdav_path(&cfg.remote_path);
    let segments: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        bail!("WebDAV 远端路径无效");
    }

    let mut parent_dirs = Vec::new();
    if segments.len() > 1 {
        let mut accum = String::new();
        for seg in &segments[..segments.len() - 1] {
            if !accum.is_empty() {
                accum.push('/');
            }
            accum.push_str(&encode_uri_segment(seg));
            let dir_url = base_url
                .join(&format!("{accum}/"))
                .context("构造 WebDAV 目录 URL 失败")?;
            parent_dirs.push(dir_url);
        }
    }

    let encoded_rel = segments
        .iter()
        .map(|s| encode_uri_segment(s))
        .collect::<Vec<_>>()
        .join("/");
    let file_url = base_url
        .join(&encoded_rel)
        .context("构造 WebDAV 文件 URL 失败")?;

    Ok((base_url, parent_dirs, file_url))
}

fn webdav_auth_headers(cfg: &WebDavSyncConfig) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if !cfg.username.trim().is_empty() || !cfg.password.is_empty() {
        let raw = format!("{}:{}", cfg.username.trim(), cfg.password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Basic {encoded}"))?,
        );
    }
    Ok(headers)
}

async fn webdav_test_connection(cfg: &WebDavSyncConfig) -> Result<SyncTestResult> {
    let (base_url, _, file_url) = build_webdav_urls(cfg)?;
    let headers = webdav_auth_headers(cfg)?;
    let client = build_http_client()?;

    // Check base endpoint with PROPFIND (Depth: 0), fallback to OPTIONS/HEAD
    let propfind = Method::from_bytes(b"PROPFIND")?;
    let mut probe_headers = headers.clone();
    probe_headers.insert(
        HeaderName::from_static("depth"),
        HeaderValue::from_static("0"),
    );

    let resp = client
        .request(propfind, base_url.clone())
        .headers(probe_headers)
        .send()
        .await
        .context("无法连接到 WebDAV 服务器")?;

    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        bail!("WebDAV 认证失败 (HTTP {})，请检查用户名与密码/应用密码", status.as_u16());
    }
    if !(status.is_success() || status.as_u16() == 207 || status == StatusCode::METHOD_NOT_ALLOWED) {
        bail!("WebDAV 服务器响应异常 (HTTP {})", status.as_u16());
    }

    // Check whether the target sync file exists
    let head_resp = client.head(file_url).headers(headers).send().await;
    let remote_exists = matches!(head_resp, Ok(r) if r.status().is_success());
    let message = if remote_exists {
        "WebDAV 连接成功（远端已存在同步配置）".to_string()
    } else {
        "WebDAV 连接成功（远端尚无配置，可点击上传）".to_string()
    };

    Ok(SyncTestResult {
        message,
        remote_exists,
    })
}

async fn webdav_put_file(cfg: &WebDavSyncConfig, payload: Vec<u8>) -> Result<()> {
    let (_, parent_dirs, file_url) = build_webdav_urls(cfg)?;
    let headers = webdav_auth_headers(cfg)?;
    let client = build_http_client()?;

    // Ensure parent directories exist via MKCOL
    let mkcol = Method::from_bytes(b"MKCOL")?;
    for dir_url in parent_dirs {
        let resp = client
            .request(mkcol.clone(), dir_url)
            .headers(headers.clone())
            .send()
            .await
            .context("创建 WebDAV 远端目录失败")?;
        let status = resp.status();
        // 201 Created, 405 Already Exists, 301/302 Redirect, 409/200 accepted
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            bail!("WebDAV 权限不足 (HTTP {})", status.as_u16());
        }
    }

    let mut put_headers = headers;
    put_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );

    let resp = client
        .put(file_url)
        .headers(put_headers)
        .body(payload)
        .send()
        .await
        .context("上传配置到 WebDAV 失败")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        bail!("WebDAV 上传失败 (HTTP {}): {}", status.as_u16(), snippet);
    }
    Ok(())
}

async fn webdav_get_file(cfg: &WebDavSyncConfig) -> Result<Option<Vec<u8>>> {
    let (_, _, file_url) = build_webdav_urls(cfg)?;
    let headers = webdav_auth_headers(cfg)?;
    let client = build_http_client()?;

    let resp = client
        .get(file_url)
        .headers(headers)
        .send()
        .await
        .context("从 WebDAV 拉取配置失败")?;

    if resp.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet: String = body.chars().take(200).collect();
        bail!("WebDAV 拉取失败 (HTTP {}): {}", status.as_u16(), snippet);
    }

    let bytes = resp.bytes().await.context("读取 WebDAV 响应内容失败")?;
    Ok(Some(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{DbDriver, SslMode};
    use chrono::TimeZone;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn sample_conn(id: &str, name: &str, host: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.to_string(),
            driver: DbDriver::Postgres,
            name: name.to_string(),
            env: "local".to_string(),
            label: "本地".to_string(),
            color: "#3b82f6".to_string(),
            host: host.to_string(),
            port: 5432,
            database: "postgres".to_string(),
            username: "postgres".to_string(),
            password: "secret".to_string(),
            file_path: None,
            ssl_mode: SslMode::Prefer,
            ssh: None,
            read_only: false,
        }
    }

    #[test]
    fn s3_url_path_style_and_virtual_hosted() {
        let mut cfg = S3SyncConfig {
            endpoint: "http://127.0.0.1:9000".into(),
            region: "us-east-1".into(),
            bucket: "my-bucket".into(),
            access_key_id: "ak".into(),
            secret_access_key: "sk".into(),
            session_token: String::new(),
            object_key: "velodb/config sync.json".into(),
            path_style: true,
        };
        let (url, uri) = build_s3_object_url(&cfg).unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:9000/my-bucket/velodb/config%20sync.json");
        assert_eq!(uri, "/my-bucket/velodb/config%20sync.json");

        cfg.endpoint = "https://s3.us-east-1.amazonaws.com".into();
        cfg.path_style = false;
        let (url, uri) = build_s3_object_url(&cfg).unwrap();
        assert_eq!(
            url.as_str(),
            "https://my-bucket.s3.us-east-1.amazonaws.com/velodb/config%20sync.json"
        );
        assert_eq!(uri, "/velodb/config%20sync.json");
    }

    #[test]
    fn s3_sigv4_headers_are_deterministic() {
        let cfg = S3SyncConfig {
            endpoint: "https://s3.amazonaws.com".into(),
            region: "us-east-1".into(),
            bucket: "examplebucket".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: String::new(),
            object_key: "test.txt".into(),
            path_style: false,
        };
        let (url, uri) = build_s3_object_url(&cfg).unwrap();
        let now = Utc.with_ymd_and_hms(2013, 5, 24, 0, 0, 0).unwrap();
        let headers = sign_s3_headers(&Method::GET, &url, &uri, &[], &cfg, now).unwrap();
        let auth = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();
        assert!(auth.starts_with(
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature="
        ));
        assert_eq!(
            headers.get("x-amz-content-sha256").unwrap().to_str().unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn webdav_urls_include_parent_dirs() {
        let cfg = WebDavSyncConfig {
            endpoint: "https://dav.example.com/dav".into(),
            username: "u".into(),
            password: "p".into(),
            remote_path: "/velodb/sub/config-sync.json".into(),
        };
        let (base, parents, file) = build_webdav_urls(&cfg).unwrap();
        assert_eq!(base.as_str(), "https://dav.example.com/dav/");
        assert_eq!(parents.len(), 2);
        assert_eq!(parents[0].as_str(), "https://dav.example.com/dav/velodb/");
        assert_eq!(parents[1].as_str(), "https://dav.example.com/dav/velodb/sub/");
        assert_eq!(
            file.as_str(),
            "https://dav.example.com/dav/velodb/sub/config-sync.json"
        );
    }

    #[test]
    fn merge_vs_overwrite_connections() {
        let local = vec![
            sample_conn("c1", "local_v1", "127.0.0.1"),
            sample_conn("c2", "local_only", "10.0.0.2"),
        ];
        let remote = vec![
            sample_conn("c1", "remote_v2", "192.168.1.10"),
            sample_conn("c3", "remote_new", "192.168.1.20"),
        ];

        let merged = merge_connections(&local, &remote, SyncPullStrategy::Merge);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].name, "remote_v2");
        assert_eq!(merged[1].id, "c3");
        assert_eq!(merged[2].id, "c2");

        let overwritten = merge_connections(&local, &remote, SyncPullStrategy::Overwrite);
        assert_eq!(overwritten.len(), 2);
        assert_eq!(overwritten[0].id, "c1");
        assert_eq!(overwritten[1].id, "c3");
    }

    async fn spawn_mock_server() -> (String, tokio::task::JoinHandle<()>) {
        use std::collections::HashMap;
        use std::sync::{Arc, Mutex};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let store = store.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    // Read headers
                    let header_end;
                    loop {
                        let n = socket.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&tmp[..n]);
                        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = pos + 4;
                            break;
                        }
                    }
                    let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
                    let mut lines = header_str.lines();
                    let req_line = lines.next().unwrap_or("");
                    let mut parts = req_line.split_whitespace();
                    let method = parts.next().unwrap_or("GET").to_string();
                    let path = parts.next().unwrap_or("/").to_string();
                    let mut content_len = 0usize;
                    for line in lines {
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                    let mut body = buf[header_end..].to_vec();
                    while body.len() < content_len {
                        let n = socket.read(&mut tmp).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&tmp[..n]);
                    }
                    body.truncate(content_len);

                    let (status, resp_body) = match method.as_str() {
                        "PROPFIND" => ("207 Multi-Status", Vec::new()),
                        "MKCOL" => ("201 Created", Vec::new()),
                        "PUT" => {
                            store.lock().unwrap().insert(path, body);
                            ("200 OK", Vec::new())
                        }
                        "HEAD" => {
                            if store.lock().unwrap().contains_key(&path) {
                                ("200 OK", Vec::new())
                            } else {
                                ("404 Not Found", Vec::new())
                            }
                        }
                        "GET" => {
                            if let Some(data) = store.lock().unwrap().get(&path).cloned() {
                                ("200 OK", data)
                            } else {
                                ("404 Not Found", Vec::new())
                            }
                        }
                        _ => ("405 Method Not Allowed", Vec::new()),
                    };

                    let resp_head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        resp_body.len()
                    );
                    let _ = socket.write_all(resp_head.as_bytes()).await;
                    let _ = socket.write_all(&resp_body).await;
                });
            }
        });

        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn end_to_end_s3_and_webdav_roundtrip() {
        let (base_url, handle) = spawn_mock_server().await;

        let mut settings = AppSettings::default();
        settings.theme = "dark".into();
        settings.default_limit = 500;
        let conns = vec![sample_conn("c1", "prod_pg", "db.example.com")];
        let mut favs = HashSet::new();
        favs.insert("c1::postgres::public::users".to_string());
        let snapshot = SyncSnapshot::build(
            "dev-test".into(),
            settings,
            conns,
            Some(vec![SqlSnippet {
                id: "s1".into(),
                name: "Ping".into(),
                sql: "SELECT 1;".into(),
            }]),
            Some(&favs),
        );

        // 1. S3 roundtrip
        let s3_cfg = SyncConfig {
            enabled: true,
            provider: SyncProvider::S3,
            s3: S3SyncConfig {
                endpoint: base_url.clone(),
                region: "us-east-1".into(),
                bucket: "velodb-sync".into(),
                access_key_id: "test-ak".into(),
                secret_access_key: "test-sk".into(),
                session_token: String::new(),
                object_key: "velodb/config.json".into(),
                path_style: true,
            },
            ..Default::default()
        };
        let test_before = test_sync_connection(&s3_cfg).await.unwrap();
        assert!(!test_before.remote_exists);
        assert!(pull_snapshot(&s3_cfg).await.unwrap().is_none());
        push_snapshot(&s3_cfg, &snapshot).await.unwrap();
        let test_after = test_sync_connection(&s3_cfg).await.unwrap();
        assert!(test_after.remote_exists);
        let pulled = pull_snapshot(&s3_cfg).await.unwrap().unwrap();
        assert_eq!(pulled.settings.theme, "dark");
        assert_eq!(pulled.settings.default_limit, 500);
        assert_eq!(pulled.connections.len(), 1);
        assert_eq!(pulled.connections[0].host, "db.example.com");

        // 2. WebDAV roundtrip
        let dav_cfg = SyncConfig {
            enabled: true,
            provider: SyncProvider::WebDav,
            webdav: WebDavSyncConfig {
                endpoint: format!("{base_url}/dav"),
                username: "alice".into(),
                password: "secret".into(),
                remote_path: "velodb/sync.json".into(),
            },
            ..Default::default()
        };
        let dav_test_before = test_sync_connection(&dav_cfg).await.unwrap();
        assert!(!dav_test_before.remote_exists);
        push_snapshot(&dav_cfg, &snapshot).await.unwrap();
        let dav_test_after = test_sync_connection(&dav_cfg).await.unwrap();
        assert!(dav_test_after.remote_exists);
        let dav_pulled = pull_snapshot(&dav_cfg).await.unwrap().unwrap();
        assert_eq!(dav_pulled.connections[0].name, "prod_pg");
        assert_eq!(dav_pulled.snippets.unwrap().len(), 1);

        handle.abort();
    }
}
