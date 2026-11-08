//! WebDAV access to one file, `<address>/tacknote/notes.json`. Reads use GET;
//! writes use PUT with If-Match, or If-None-Match: * for the first upload, so a
//! write that races another device is detected instead of overwriting it.
//! Some servers ignore both (SFTPGo did, 2026-11); then the overwritten
//! device puts its notes back on its next sync (see merge.rs), and
//! `TACKNOTE_SELFTEST=webdav` shows which kind a server is (server_check.rs).

use std::fmt;
use std::future::Future;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH};
use reqwest::{Client, Method, Response, StatusCode, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncErrorKind {
    Auth,
    CredentialsUnavailable,
    BadUrl,
    Network,
    Server,
    Corrupt,
    UnsupportedVersion,
    /// The synced notes could not be saved on this device.
    LocalStorage,
}

/// `detail` is for logs and never contains the password.
#[derive(Debug)]
pub struct SyncError {
    pub kind: SyncErrorKind,
    pub detail: String,
}

impl SyncError {
    pub fn new(kind: SyncErrorKind, detail: impl Into<String>) -> SyncError {
        SyncError { kind, detail: detail.into() }
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.detail)
    }
}

impl std::error::Error for SyncError {}

/// Why a write did not happen.
#[derive(Debug)]
pub enum WriteError {
    /// The file changed on the server between our read and our write.
    Changed,
    Failed(SyncError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteFile {
    pub body: String,
    pub etag: Option<String>,
}

/// Where the notes file lives. The WebDAV store is the real one; tests use
/// others.
pub trait RemoteStore: Send + Sync {
    /// `None` when nothing has been uploaded yet.
    fn read(&self) -> impl Future<Output = Result<Option<RemoteFile>, SyncError>> + Send;

    /// `previous` is what `read` returned, so a concurrent write is detected.
    fn write(&self, body: String, previous: Option<RemoteFile>) -> impl Future<Output = Result<(), WriteError>> + Send;
}

#[derive(Clone, PartialEq, Eq)]
pub struct WebDavConfig {
    pub url: String,
    pub username: String,
    pub password: String,
}

impl fmt::Debug for WebDavConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebDavConfig")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

impl WebDavConfig {
    pub fn is_complete(&self) -> bool {
        !self.url.trim().is_empty() && !self.username.trim().is_empty()
    }

    /// Plain http to another computer: the password and notes travel unencrypted.
    pub fn is_insecure(&self) -> bool {
        match Url::parse(self.url.trim()) {
            Ok(url) => url.scheme() == "http" && !matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
            Err(_) => false,
        }
    }
}

const TIMEOUT: Duration = Duration::from_secs(20);
const FOLDER: &str = "tacknote/";
const FILE: &str = "notes.json";

/// One client for the whole app, so connections are reused and closed.
pub fn http_client() -> Client {
    Client::builder()
        .timeout(TIMEOUT)
        .build()
        .expect("the HTTP client settings are valid")
}

pub struct WebDavStore {
    config: WebDavConfig,
    client: Client,
}

impl WebDavStore {
    pub fn new(config: WebDavConfig, client: Client) -> WebDavStore {
        WebDavStore { config, client }
    }

    fn folder_url(&self) -> Result<Url, SyncError> {
        let trimmed = self.config.url.trim();
        let with_slash = if trimmed.ends_with('/') { trimmed.to_string() } else { format!("{trimmed}/") };
        let base = Url::parse(&with_slash).map_err(|_| SyncError::new(SyncErrorKind::BadUrl, "not a valid address"))?;
        if !matches!(base.scheme(), "http" | "https") {
            return Err(SyncError::new(SyncErrorKind::BadUrl, "the address must start with https://"));
        }
        base.join(FOLDER).map_err(|_| SyncError::new(SyncErrorKind::BadUrl, "not a valid address"))
    }

    fn file_url(&self) -> Result<Url, SyncError> {
        self.folder_url()?
            .join(FILE)
            .map_err(|_| SyncError::new(SyncErrorKind::BadUrl, "not a valid address"))
    }

    async fn send(&self, method: Method, url: Url, build: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder) -> Result<Response, SyncError> {
        let request = self
            .client
            .request(method, url)
            .basic_auth(&self.config.username, Some(&self.config.password));
        build(request).send().await.map_err(network)
    }

    async fn create_folder(&self) -> Result<(), SyncError> {
        let mkcol = Method::from_bytes(b"MKCOL").expect("MKCOL is a valid method");
        let response = self.send(mkcol, self.folder_url()?, |request| request).await?;
        // 405 means the folder already exists.
        if response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        check(&response)
    }
}

impl RemoteStore for WebDavStore {
    async fn read(&self) -> Result<Option<RemoteFile>, SyncError> {
        let response = self.send(Method::GET, self.file_url()?, |request| request).await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        check(&response)?;
        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = response.bytes().await.map_err(network)?;
        let body = String::from_utf8(bytes.to_vec())
            .map_err(|_| SyncError::new(SyncErrorKind::Corrupt, "the file is not UTF-8 text"))?;
        Ok(Some(RemoteFile { body, etag }))
    }

    async fn write(&self, body: String, previous: Option<RemoteFile>) -> Result<(), WriteError> {
        let url = self.file_url().map_err(WriteError::Failed)?;
        let put = |request: reqwest::RequestBuilder| {
            let request = request.header(CONTENT_TYPE, "application/json; charset=utf-8").body(body.clone());
            match &previous {
                None => request.header(IF_NONE_MATCH, "*"),
                Some(RemoteFile { etag: Some(etag), .. }) => request.header(IF_MATCH, etag.as_str()),
                Some(RemoteFile { etag: None, .. }) => request,
            }
        };
        let mut response = self.send(Method::PUT, url.clone(), put).await.map_err(WriteError::Failed)?;
        if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::CONFLICT) {
            self.create_folder().await.map_err(WriteError::Failed)?;
            response = self.send(Method::PUT, url, put).await.map_err(WriteError::Failed)?;
        }
        if response.status() == StatusCode::PRECONDITION_FAILED {
            return Err(WriteError::Changed);
        }
        check(&response).map_err(WriteError::Failed)
    }
}

/// The URL is dropped from the message, in case it carries a password.
fn network(error: reqwest::Error) -> SyncError {
    SyncError::new(SyncErrorKind::Network, error.without_url().to_string())
}

fn check(response: &Response) -> Result<(), SyncError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let kind = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => SyncErrorKind::Auth,
        _ => SyncErrorKind::Server,
    };
    Err(SyncError::new(kind, format!("HTTP {}", status.as_u16())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(url: &str) -> WebDavConfig {
        WebDavConfig { url: url.into(), username: "me".into(), password: "secret".into() }
    }

    #[test]
    fn debug_output_hides_the_password() {
        let shown = format!("{:?}", config("https://example.test/"));
        assert!(!shown.contains("secret"));
        assert!(shown.contains("***"));
    }

    #[test]
    fn plain_http_to_another_computer_is_insecure() {
        assert!(config("http://example.test/dav").is_insecure());
        assert!(!config("https://example.test/dav").is_insecure());
        assert!(!config("http://localhost:8080/").is_insecure());
    }

    #[test]
    fn the_notes_file_sits_in_a_tacknote_folder() {
        let store = WebDavStore::new(config("https://example.test/dav"), http_client());
        assert_eq!(store.file_url().unwrap().as_str(), "https://example.test/dav/tacknote/notes.json");
    }

    #[test]
    fn a_malformed_address_is_a_bad_url() {
        for url in ["http://[bad", "ftp://example.test/", "not a url"] {
            let store = WebDavStore::new(config(url), http_client());
            assert_eq!(store.file_url().unwrap_err().kind, SyncErrorKind::BadUrl, "{url}");
        }
    }
}
