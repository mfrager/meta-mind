//! `http.fetch`: reach one allowlisted host, over a transport this build actually has.
//!
//! Two honest limits, both stated rather than worked around:
//!
//! * **The host must be on the granted allowlist.** [`ExecCtx::check_host`] is exact and
//!   case-insensitive; a suffix rule would let `evil-example.com` through a grant on
//!   `example.com`, which is the oldest bug in allowlisting.
//! * **Only `http://` is spoken.** This build links no TLS stack, so an `https://` URL
//!   is refused rather than downgraded: silently fetching in the clear a resource the
//!   caller asked to be encrypted is exactly the kind of quiet substitution the phase's
//!   "never fake an outcome" invariant exists to prevent. When a TLS client is added,
//!   this tool gains a scheme, not a rewrite.
//!
//! The request is composed by hand — request line, `Host`, `Connection: close` — because
//! the tool needs no more than that and an HTTP library would add a dependency surface
//! (§7 of the parent plan records a copied-in dependency with its revision) for one GET.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::ToolError;
use crate::permissions::{PermissionAction, PermissionReq};
use crate::registry::{ExecCtx, Tool, ToolArgs, ToolOutcome};
use crate::sandbox::SandboxTier;
use crate::spec::{
    JsonSchema, Reversibility, SideEffectClass, ToolAnnotations, ToolName, ToolSpec,
};

/// How long a fetch may take before the connection is abandoned.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// A URL split into the parts the tool needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedUrl {
    /// The scheme, lowercase.
    pub scheme: String,
    /// The host, as written.
    pub host: String,
    /// The port, defaulted from the scheme.
    pub port: u16,
    /// The path and query, with a leading `/`.
    pub path: String,
}

/// Parse an absolute URL.
pub fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("{url:?} has no scheme"))?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme.is_empty() {
        return Err(format!("{url:?} has no scheme"));
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(format!("{url:?} has no host"));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') => {
            let port: u16 = port
                .parse()
                .map_err(|_| format!("{port:?} is not a port number"))?;
            (host.to_string(), port)
        }
        _ => (
            authority.to_string(),
            if scheme == "https" { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return Err(format!("{url:?} has no host"));
    }
    if path.contains(char::is_whitespace) {
        return Err("the URL contains whitespace".to_string());
    }
    Ok(ParsedUrl {
        scheme,
        host,
        port,
        path: path.to_string(),
    })
}

/// The status code and body of a response, split at the first blank line.
pub fn split_response(response: &str) -> (u16, &str) {
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((response, ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status, body)
}

/// `http.fetch`.
pub struct HttpFetchTool {
    spec: ToolSpec,
}

impl Default for HttpFetchTool {
    fn default() -> Self {
        HttpFetchTool::new()
    }
}

impl HttpFetchTool {
    /// The tool.
    pub fn new() -> Self {
        HttpFetchTool {
            spec: ToolSpec {
                name: ToolName::new("http.fetch").expect("http.fetch is a namespace.verb pair"),
                version: "0.1.0".to_string(),
                description: "Fetch an allowlisted http:// URL and return its status and body."
                    .to_string(),
                input_schema: JsonSchema::object_with_strings(&["url"]),
                output_schema: JsonSchema::any_object(),
                permissions: vec![PermissionReq::new(
                    "net:connect:api.metamind.dev",
                    PermissionAction::Connect,
                )],
                reversibility: Reversibility::Compensatable,
                side_effects: SideEffectClass::External,
                // Not `read_only`: fetching reaches outside the kernel, so the hint would
                // claim a purity the side-effect class denies, and the registry refuses
                // that contradiction rather than believing one of the two.
                annotations: ToolAnnotations {
                    read_only: false,
                    destructive: false,
                    idempotent: false,
                    open_world: true,
                },
                module_uri: "https://metamind.dev/code/module/tools/http-fetch".to_string(),
                sandbox_tier: SandboxTier::WasmCaps,
            },
        }
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn invoke(&self, ctx: &ExecCtx, _args: ToolArgs) -> Result<ToolOutcome, ToolError> {
        let url = ctx.arg_str("url")?;
        let parsed = parse_url(&url).map_err(|reason| ctx.bad_argument("url", reason))?;
        if parsed.scheme == "https" {
            return Err(ctx.unsupported(
                "https is refused: this build links no TLS stack, and fetching a resource \
                 the caller asked to be encrypted in the clear is not a substitution it will \
                 make",
            ));
        }
        if parsed.scheme != "http" {
            return Err(ctx.bad_argument(
                "url",
                format!("scheme {:?} is not supported", parsed.scheme),
            ));
        }
        ctx.check_host(&parsed.host)?;

        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: metamind/0.1\r\nAccept: */*\r\n\
             Connection: close\r\n\r\n",
            parsed.path, parsed.host
        );
        let stream = tokio::time::timeout(
            FETCH_TIMEOUT,
            tokio::net::TcpStream::connect((parsed.host.as_str(), parsed.port)),
        )
        .await
        .map_err(|_| ctx.failed(format!("connecting to {} timed out", parsed.host)))?
        .map_err(|e| ctx.failed(format!("cannot connect to {}: {e}", parsed.host)))?;
        let (mut reader, mut writer) = stream.into_split();
        writer
            .write_all(request.as_bytes())
            .await
            .map_err(|e| ctx.failed(format!("cannot send the request: {e}")))?;
        let mut response = Vec::new();
        let limit = ctx.guard.capabilities().max_bytes;
        tokio::time::timeout(
            FETCH_TIMEOUT,
            (&mut reader).take(limit).read_to_end(&mut response),
        )
        .await
        .map_err(|_| ctx.failed("reading the response timed out"))?
        .map_err(|e| ctx.failed(format!("cannot read the response: {e}")))?;
        ctx.check_size(response.len() as u64)?;

        let text = String::from_utf8_lossy(&response).to_string();
        let (status, body) = split_response(&text);
        let output = json!({
            "url": url,
            "host": parsed.host,
            "status": status,
            "bytes": response.len(),
            "sha256": mm_core::content_hash(&response),
            "body": body,
        });
        let summary = format!("GET {} returned {status}", parsed.host);
        Ok(ctx.outcome(output, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::ActionSpec;
    use crate::permissions::Principal;
    use crate::sandbox::{CapabilityGuard, SandboxCapabilities, DEFAULT_MAX_BYTES};
    use mm_core::{Timestamp, Ulid};

    async fn context(permissions: Vec<PermissionReq>, args: serde_json::Value) -> ExecCtx {
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let sql = mm_store_sqlite::SqliteStore::open(&dir.path().join("mm.db"))
            .await
            .unwrap();
        sql.migrate().await.unwrap();
        let shapes = mm_core::Config::repo_root()
            .join("ontology")
            .join("shapes")
            .join("epistemic_shapes.ttl");
        let graph = Box::leak(Box::new(
            mm_store_graph::GraphStore::in_memory(&shapes)
                .await
                .unwrap(),
        ));
        let spec = HttpFetchTool::new().spec;
        let guard = CapabilityGuard::new(
            SandboxCapabilities::from_permissions(&permissions, dir.path(), DEFAULT_MAX_BYTES),
            SandboxTier::WasmCaps,
            dir.path(),
        );
        ExecCtx {
            action_id: Ulid::from_parts(1_700_000_000_000, 11),
            trace_id: Ulid::from_parts(1_700_000_000_000, 12),
            action: ActionSpec::new(spec.name.clone(), args, Principal::system()),
            spec,
            guard,
            sql,
            graph: graph.handle().clone(),
            workdir: dir.path().to_path_buf(),
            started_at: Timestamp::now(),
            confirmed: false,
        }
    }

    fn net(hosts: &str) -> PermissionReq {
        PermissionReq::new(format!("net:connect:{hosts}"), PermissionAction::Connect)
    }

    #[tokio::test]
    async fn a_host_outside_the_allowlist_is_refused_before_any_connection() {
        let tool = HttpFetchTool::new();
        let ctx = context(
            vec![net("api.metamind.dev")],
            json!({ "url": "http://example.com/" }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.is_sandbox_denial(), "{error}");
        assert!(error.to_string().contains("allowlist"), "{error}");
    }

    #[tokio::test]
    async fn a_caller_with_no_network_grant_cannot_fetch() {
        let tool = HttpFetchTool::new();
        let ctx = context(Vec::new(), json!({ "url": "http://api.metamind.dev/x" })).await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert!(error.to_string().contains("allowlist"), "{error}");
    }

    #[tokio::test]
    async fn https_is_refused_rather_than_downgraded() {
        let tool = HttpFetchTool::new();
        let ctx = context(
            vec![net("api.metamind.dev")],
            json!({ "url": "https://api.metamind.dev/x" }),
        )
        .await;
        let error = tool.invoke(&ctx, ToolArgs::empty()).await.unwrap_err();
        assert_eq!(error.code(), "tool.unsupported");
        assert!(error.to_string().contains("TLS"), "{error}");
    }

    #[test]
    fn urls_parse() {
        assert_eq!(
            parse_url("http://api.metamind.dev/a/b?c=1").unwrap(),
            ParsedUrl {
                scheme: "http".into(),
                host: "api.metamind.dev".into(),
                port: 80,
                path: "/a/b?c=1".into(),
            }
        );
        assert_eq!(parse_url("http://h").unwrap().path, "/");
        assert_eq!(parse_url("http://h:8080/x").unwrap().port, 8080);
        assert_eq!(parse_url("https://h/x").unwrap().port, 443);
        assert!(parse_url("api.metamind.dev").is_err());
        assert!(parse_url("http://").is_err());
        assert!(parse_url("http://h:notaport/x").is_err());
    }

    #[test]
    fn a_response_splits_at_the_blank_line() {
        let (status, body) = split_response("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
        assert_eq!(status, 200);
        assert_eq!(body, "hi");
        assert_eq!(split_response("garbage").0, 0);
    }
}
