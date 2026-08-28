//! Asking a container registry what the newest version of an image is.
//!
//! One question, answered without downloading anything: *does the tag this
//! container runs point at a different image than the one it is running?* That
//! is a digest comparison — the registry's current digest for the tag against
//! the digest the daemon recorded when it pulled — and it needs a `HEAD` on one
//! manifest endpoint, not a pull.
//!
//! # What this speaks
//!
//! The [OCI Distribution / Docker Registry HTTP API v2] manifest endpoint,
//! `HEAD /v2/<repository>/manifests/<reference>`, whose response carries the
//! digest in the `Docker-Content-Digest` header. Authentication is discovered
//! rather than configured: an unauthenticated request that needs a token comes
//! back `401` with an RFC 7235 `WWW-Authenticate: Bearer realm=…,service=…`
//! challenge, which names the token service to ask. Following the challenge is
//! what makes one implementation work against every registry that implements
//! the spec — Docker Hub, GHCR, and a registry that needs no token at all all
//! take the same code path, and none of their hostnames appear in it.
//!
//! # What this does not do
//!
//! **Private registries and private repositories are out of scope for this
//! pass.** The token request is always made anonymously, because there is
//! nowhere yet to put a credential: the connector's configuration has no
//! registry credential field, and adding one is a decision about secret storage
//! rather than about HTTP. A private repository therefore answers the challenge
//! with a `401`, which surfaces as [`ConnectorError::AuthFailed`] naming the
//! repository — a clear "this needs credentials Loom does not have", not a
//! silent "no update available". See
//! `docs/adr/0023-docker-update-management.md`.
//!
//! [OCI Distribution / Docker Registry HTTP API v2]: https://distribution.github.io/distribution/spec/api/

use std::time::Duration;

use loom_core::connector::ConnectorError;

/// Registry host used when an image reference names none.
///
/// The *API* host, which is not the same string as the token service's `service`
/// parameter (`registry.docker.io`) — a distinction that only matters because
/// the challenge supplies the latter and this constant supplies the former.
const DEFAULT_REGISTRY_HOST: &str = "registry-1.docker.io";

/// Namespace prefix Docker Hub gives its own images.
///
/// `nginx:latest` is `library/nginx` on the wire; every other registry takes the
/// repository exactly as written.
const DEFAULT_NAMESPACE: &str = "library";

/// Tag assumed when a reference names none, per Docker's own resolution rules.
const DEFAULT_TAG: &str = "latest";

/// Manifest media types sent in `Accept`.
///
/// All four, because a registry returns whichever the image actually is and a
/// request that accepts only one gets a `404` or a `415` for an image that is
/// the other. The two index/list types matter most: a modern multi-architecture
/// image *is* an index, and its digest is the one a daemon records.
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
                               application/vnd.docker.distribution.manifest.list.v2+json, \
                               application/vnd.oci.image.manifest.v1+json, \
                               application/vnd.docker.distribution.manifest.v2+json";

/// How long any single registry request may take.
///
/// A registry is a third party over the internet, reached from a background
/// scheduler. A check that hangs is worse than one that fails: the failure is
/// reportable.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// One image reference, split into the parts a registry request needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    /// Registry API host, defaulted to Docker Hub's.
    pub registry: String,
    /// Repository path as the registry expects it, `library/` included for a
    /// bare Docker Hub name.
    pub repository: String,
    /// Tag, defaulted to `latest`.
    pub tag: String,
}

impl ImageReference {
    /// Splits an image reference the way Docker's own resolution rules do.
    ///
    /// The one genuinely ambiguous case is the first path segment: in
    /// `example.com/app` it is a registry and in `owner/app` it is a namespace.
    /// The rule — a first segment containing a `.` or a `:`, or the literal
    /// `localhost`, is a registry — is Docker's, and it is why `owner/app`
    /// resolves to Docker Hub while `owner.com/app` does not.
    ///
    /// A reference already pinned to a digest (`image@sha256:…`), or one that is
    /// a bare image **id** (`sha256:0123…`, which is what a container created
    /// from an untagged image reports), has no update question to ask: each
    /// names one immutable image. Those return `None` rather than an error,
    /// because "there is nothing to check here" is a correct answer, not a
    /// failure — and treating an id as a repository would send a query for
    /// `library/sha256` to a registry and report the refusal as though the user
    /// had a private repository.
    pub fn parse(reference: &str) -> Option<Self> {
        let reference = reference.trim();
        if reference.is_empty() || reference.contains('@') || is_image_id(reference) {
            return None;
        }

        let (host, remainder) = match reference.split_once('/') {
            Some((first, rest))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_owned(), rest.to_owned())
            }
            _ => (DEFAULT_REGISTRY_HOST.to_owned(), reference.to_owned()),
        };

        // Only a colon *after* the last slash is a tag separator; one before it
        // is a registry port, which is why the split happens on the final path
        // segment rather than on the whole string.
        let (path, tag) = match remainder.rsplit_once(':') {
            Some((path, tag)) if !tag.contains('/') && !tag.is_empty() => {
                (path.to_owned(), tag.to_owned())
            }
            _ => (remainder, DEFAULT_TAG.to_owned()),
        };
        if path.is_empty() {
            return None;
        }

        let repository = if host == DEFAULT_REGISTRY_HOST && !path.contains('/') {
            format!("{DEFAULT_NAMESPACE}/{path}")
        } else {
            path
        };

        Some(Self {
            registry: host,
            repository,
            tag,
        })
    }

    /// The manifest endpoint for this reference.
    pub fn manifest_url(&self) -> String {
        format!(
            "https://{}/v2/{}/manifests/{}",
            self.registry, self.repository, self.tag
        )
    }
}

/// Whether a reference is a bare image id rather than a repository and tag.
///
/// `sha256:` followed by hex, which is how the daemon reports the image of a
/// container created from something that had no tag — after a digest pull, or
/// from a locally built image that was never named.
fn is_image_id(reference: &str) -> bool {
    reference
        .split_once(':')
        .is_some_and(|(algorithm, digest)| {
            algorithm.eq_ignore_ascii_case("sha256")
                && digest.len() >= 32
                && digest
                    .chars()
                    .all(|character| character.is_ascii_hexdigit())
        })
}

/// A bearer-token challenge, as parsed from a `WWW-Authenticate` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenChallenge {
    /// Token service endpoint the registry named.
    pub realm: String,
    /// `service` parameter to send with the token request, when given.
    pub service: Option<String>,
    /// `scope` parameter to send with the token request, when given.
    pub scope: Option<String>,
}

impl TokenChallenge {
    /// Reads a `Bearer realm="…",service="…",scope="…"` challenge.
    ///
    /// Returns `None` for a challenge that is not `Bearer` or that names no
    /// realm — there is nowhere to send a token request, and guessing one from
    /// the registry host is how an implementation ends up working for exactly
    /// the registry it was tested against.
    pub fn parse(header: &str) -> Option<Self> {
        let parameters = header.strip_prefix("Bearer ")?;
        let mut realm = None;
        let mut service = None;
        let mut scope = None;

        for parameter in parameters.split(',') {
            let (key, value) = parameter.split_once('=')?;
            let value = value.trim().trim_matches('"').to_owned();
            match key.trim() {
                "realm" => realm = Some(value),
                "service" => service = Some(value),
                "scope" => scope = Some(value),
                _ => {}
            }
        }

        Some(Self {
            realm: realm?,
            service,
            scope,
        })
    }

    /// The token request URL, with the challenge's own parameters carried
    /// through unchanged.
    pub fn token_url(&self) -> String {
        let mut url = self.realm.clone();
        let mut separator = if url.contains('?') { '&' } else { '?' };
        if let Some(service) = &self.service {
            url.push(separator);
            url.push_str(&format!("service={}", encode(service)));
            separator = '&';
        }
        if let Some(scope) = &self.scope {
            url.push(separator);
            url.push_str(&format!("scope={}", encode(scope)));
        }
        url
    }
}

/// Percent-encodes the characters that appear in registry scopes and service
/// names and mean something else in a query string.
///
/// Not a general URL encoder: the inputs are registry hostnames and scope
/// strings like `repository:library/nginx:pull`, whose alphabet is known.
fn encode(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            ' ' => "%20".to_owned(),
            '&' => "%26".to_owned(),
            '#' => "%23".to_owned(),
            '+' => "%2B".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// What one manifest request came back as, reduced to what the flow needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestProbe {
    /// HTTP status.
    pub status: u16,
    /// `Docker-Content-Digest`, when the registry sent one.
    pub digest: Option<String>,
    /// `WWW-Authenticate`, when the registry sent one.
    pub challenge: Option<String>,
}

/// The HTTP calls a digest check makes.
///
/// A trait so the flow above it — challenge, token, retry, read the digest — can
/// be tested against canned responses. The alternative is a test that reaches a
/// real registry, which would make the suite depend on someone else's uptime,
/// consume their rate limit, and be unable to reproduce the failures that
/// matter most (a 429, a 404, a malformed challenge) at all.
#[async_trait::async_trait]
pub trait RegistryTransport: Send + Sync {
    /// `HEAD` on a manifest URL, optionally bearing a token.
    async fn head_manifest(
        &self,
        url: &str,
        token: Option<&str>,
    ) -> Result<ManifestProbe, ConnectorError>;

    /// Fetches a bearer token from a token service, returning its `token` field.
    async fn fetch_token(&self, url: &str) -> Result<String, ConnectorError>;
}

/// Builds the real HTTPS transport.
///
/// Exposed so an integration test can drive the same client the connector uses
/// rather than a second one built to the same understanding.
pub fn http_registry() -> Result<std::sync::Arc<dyn RegistryTransport>, ConnectorError> {
    HttpRegistry::new()
        .map(|client| std::sync::Arc::new(client) as std::sync::Arc<dyn RegistryTransport>)
}

/// The real transport, over HTTPS.
pub struct HttpRegistry {
    client: reqwest::Client,
}

impl HttpRegistry {
    /// Builds a client with the shared timeout applied.
    pub fn new() -> Result<Self, ConnectorError> {
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("loom-connector-docker/", env!("CARGO_PKG_VERSION")))
            .build()
            .map(|client| Self { client })
            .map_err(|error| {
                ConnectorError::Internal(format!("could not build an HTTPS client: {error}"))
            })
    }
}

#[async_trait::async_trait]
impl RegistryTransport for HttpRegistry {
    async fn head_manifest(
        &self,
        url: &str,
        token: Option<&str>,
    ) -> Result<ManifestProbe, ConnectorError> {
        let mut request = self.client.head(url).header("Accept", MANIFEST_ACCEPT);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }

        let response = request.send().await.map_err(|error| {
            ConnectorError::unreachable(format!("could not reach the registry: {}", reason(&error)))
        })?;

        Ok(ManifestProbe {
            status: response.status().as_u16(),
            digest: header(&response, "docker-content-digest"),
            challenge: header(&response, "www-authenticate"),
        })
    }

    async fn fetch_token(&self, url: &str) -> Result<String, ConnectorError> {
        let response = self.client.get(url).send().await.map_err(|error| {
            ConnectorError::unreachable(format!(
                "could not reach the registry's token service: {}",
                reason(&error)
            ))
        })?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(ConnectorError::AuthFailed {
                reason: format!("the registry's token service answered {status}"),
            });
        }

        parse_token(&body)
    }
}

/// Reads one header as a string, or `None` when it is absent or not text.
fn header(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// A short description of a transport failure.
///
/// `reqwest::Error`'s own `Display` includes the full URL, which would put a
/// registry path into every error message; the distinction that matters to a
/// user is timeout-versus-connect-versus-TLS.
fn reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the request timed out".to_owned()
    } else if error.is_connect() {
        "the connection failed".to_owned()
    } else {
        "the request failed".to_owned()
    }
}

/// Reads the `token` (or OAuth2 `access_token`) field of a token response.
pub fn parse_token(body: &str) -> Result<String, ConnectorError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| {
        ConnectorError::Internal(format!(
            "the registry's token service returned something that is not JSON: {error}"
        ))
    })?;

    value
        .get("token")
        .or_else(|| value.get("access_token"))
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ConnectorError::AuthFailed {
            reason: "the registry's token service returned no token".to_owned(),
        })
}

/// Turns a non-success manifest status into the error a user can act on.
///
/// The distinctions are the point. "Rate-limited" means *wait*, and it is the
/// one failure a homelab checking a dozen images will actually hit; "not found"
/// means the tag is gone or misspelled; "needs credentials" means the
/// repository is private, which this pass cannot do anything about and must
/// therefore say plainly. Collapsing them into one message would leave every
/// one of those users with the same useless sentence.
pub fn manifest_error(status: u16, reference: &ImageReference) -> ConnectorError {
    let repository = format!("{}/{}", reference.registry, reference.repository);
    match status {
        401 | 403 => ConnectorError::AuthFailed {
            reason: format!(
                "the registry refused anonymous access to {repository}. Loom does not yet \
                 support registry credentials, so private repositories cannot be checked"
            ),
        },
        404 => ConnectorError::unreachable(format!(
            "the registry has no tag `{}` in {repository}",
            reference.tag
        )),
        429 => ConnectorError::unreachable(format!(
            "the registry is rate-limiting this host, so {repository} could not be checked \
             right now. Checks resume on their own; raising the check interval makes this \
             less likely"
        )),
        other => {
            ConnectorError::unreachable(format!("the registry answered {other} for {repository}"))
        }
    }
}

/// Asks a registry for the current digest of an image reference's tag.
///
/// One unauthenticated `HEAD`; if that draws a challenge, one token request and
/// one retry. Never more than that: a registry that answers `401` to a request
/// carrying the token it just issued is not going to be talked round by a third
/// attempt, and a loop here would be a loop against someone else's rate limit.
pub async fn current_digest(
    transport: &dyn RegistryTransport,
    reference: &ImageReference,
) -> Result<String, ConnectorError> {
    let url = reference.manifest_url();
    let probe = transport.head_manifest(&url, None).await?;

    let probe = if probe.status == 401 {
        let challenge = probe
            .challenge
            .as_deref()
            .and_then(TokenChallenge::parse)
            .ok_or_else(|| ConnectorError::AuthFailed {
                reason: format!(
                    "the registry at {} requires authentication but did not say where to \
                     obtain a token",
                    reference.registry
                ),
            })?;
        let token = transport.fetch_token(&challenge.token_url()).await?;
        transport.head_manifest(&url, Some(&token)).await?
    } else {
        probe
    };

    if !(200..300).contains(&probe.status) {
        return Err(manifest_error(probe.status, reference));
    }

    probe.digest.ok_or_else(|| {
        ConnectorError::Internal(format!(
            "the registry answered for {}/{} without a Docker-Content-Digest header",
            reference.registry, reference.repository
        ))
    })
}

/// Whether a container is running something other than what its tag now points
/// at.
///
/// `repo_digests` is the daemon's own record — `RepoDigests` from an image
/// inspect — in `repository@sha256:…` form. The comparison is deliberately
/// against *any* of them rather than a single expected entry: one local image
/// can carry digests for several repositories it was tagged into, and a match
/// on any of them means the registry is serving what is already here.
///
/// An image with no repo digests at all — built locally, never pushed — is
/// reported as up to date rather than as an update: there is no registry
/// version of it to be behind. (The caller skips the registry query entirely in
/// that case; this stays correct in its own right.)
pub fn is_outdated(repo_digests: &[String], registry_digest: &str) -> bool {
    if repo_digests.is_empty() {
        return false;
    }

    !repo_digests.iter().any(|entry| {
        entry
            .rsplit_once('@')
            .map(|(_, digest)| digest == registry_digest)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_the_way_docker_resolves_them() {
        let cases = [
            ("nginx", "registry-1.docker.io", "library/nginx", "latest"),
            (
                "nginx:1.27",
                "registry-1.docker.io",
                "library/nginx",
                "1.27",
            ),
            ("owner/app:v2", "registry-1.docker.io", "owner/app", "v2"),
            ("ghcr.io/owner/app:v2", "ghcr.io", "owner/app", "v2"),
            // A first segment with a dot is a host, which is the whole reason
            // `owner/app` and `owner.com/app` resolve differently.
            (
                "registry.example.com/team/app",
                "registry.example.com",
                "team/app",
                "latest",
            ),
            // A port is a colon that is not a tag separator.
            (
                "registry.example.com:5000/team/app:dev",
                "registry.example.com:5000",
                "team/app",
                "dev",
            ),
            ("localhost:5000/app", "localhost:5000", "app", "latest"),
            // Deep paths stay whole; only Docker Hub gets a namespace added.
            (
                "ghcr.io/owner/group/app:1",
                "ghcr.io",
                "owner/group/app",
                "1",
            ),
        ];

        for (reference, registry, repository, tag) in cases {
            let parsed = ImageReference::parse(reference)
                .unwrap_or_else(|| panic!("{reference} should parse"));
            assert_eq!(
                parsed,
                ImageReference {
                    registry: registry.to_owned(),
                    repository: repository.to_owned(),
                    tag: tag.to_owned(),
                },
                "unexpected split for {reference}"
            );
        }

        assert_eq!(
            ImageReference::parse("nginx:1.27").unwrap().manifest_url(),
            "https://registry-1.docker.io/v2/library/nginx/manifests/1.27"
        );
    }

    #[test]
    fn a_digest_pinned_or_empty_reference_has_no_update_question() {
        // Already immutable: there is no newer version of one exact image.
        assert_eq!(ImageReference::parse("nginx@sha256:0123456789abcdef"), None);
        assert_eq!(ImageReference::parse(""), None);
        assert_eq!(ImageReference::parse("   "), None);

        // A bare image id, which is what a container created from an untagged
        // image reports. Read as a repository it becomes `library/sha256` with
        // the hex as a tag — a real query, to a real registry, whose refusal
        // gets reported to the user as a private repository they do not have.
        // Observed against a live daemon before this was guarded.
        assert_eq!(
            ImageReference::parse(
                "sha256:83b2b6703a620bf2e001ab57f7adc414d891787b3c59859b1b62909e48dd2242"
            ),
            None
        );
        // ...but a repository that merely begins with those characters is not
        // an id, and must still resolve.
        assert!(ImageReference::parse("sha256scanner/app:1").is_some());
    }

    #[test]
    fn a_bearer_challenge_names_where_to_get_a_token() {
        // The exact shape Docker Hub answers a bare manifest request with.
        let challenge = TokenChallenge::parse(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull""#,
        )
        .expect("a bearer challenge");
        assert_eq!(challenge.realm, "https://auth.docker.io/token");
        assert_eq!(challenge.service.as_deref(), Some("registry.docker.io"));
        assert_eq!(
            challenge.token_url(),
            "https://auth.docker.io/token?service=registry.docker.io\
             &scope=repository:library/alpine:pull"
        );

        // A challenge with no realm names nowhere to ask, and guessing one from
        // the hostname is how this ends up working for one registry only.
        assert_eq!(TokenChallenge::parse(r#"Bearer service="x""#), None);
        assert_eq!(TokenChallenge::parse("Basic realm=\"x\""), None);
    }

    #[test]
    fn a_token_response_is_read_from_either_field_name() {
        assert_eq!(parse_token(r#"{"token":"abc"}"#).unwrap(), "abc");
        // OAuth2-style services answer with the other name.
        assert_eq!(
            parse_token(r#"{"access_token":"def","expires_in":300}"#).unwrap(),
            "def"
        );
        assert!(matches!(
            parse_token(r#"{"expires_in":300}"#),
            Err(ConnectorError::AuthFailed { .. })
        ));
        assert!(matches!(
            parse_token("<html>nope</html>"),
            Err(ConnectorError::Internal(_))
        ));
    }

    #[test]
    fn each_registry_failure_says_something_different() {
        let reference = ImageReference::parse("owner/private:v1").unwrap();

        let private = manifest_error(401, &reference).to_string();
        assert!(private.contains("private repositories"), "{private}");

        let missing = manifest_error(404, &reference).to_string();
        assert!(missing.contains("no tag `v1`"), "{missing}");

        // The one every homelab will actually hit, and the one whose remedy —
        // wait, or check less often — is nothing like the others'.
        let limited = manifest_error(429, &reference).to_string();
        assert!(limited.contains("rate-limiting"), "{limited}");
        assert!(limited.contains("check interval"), "{limited}");

        assert!(manifest_error(503, &reference)
            .to_string()
            .contains("answered 503"));
    }

    #[test]
    fn a_digest_matches_any_repository_the_local_image_was_tagged_into() {
        let digest = "sha256:aaaa";
        assert!(!is_outdated(
            &["example/app@sha256:aaaa".to_owned()],
            digest
        ));
        assert!(is_outdated(&["example/app@sha256:bbbb".to_owned()], digest));
        // One local image, several repositories: a match on any of them means
        // the registry is serving what is already here.
        assert!(!is_outdated(
            &[
                "mirror.example/app@sha256:bbbb".to_owned(),
                "example/app@sha256:aaaa".to_owned(),
            ],
            digest
        ));
        // Built locally and never pushed: there is no registry version of it to
        // be behind, so it is not "an update available".
        assert!(!is_outdated(&[], digest));
        assert!(is_outdated(&["malformed-entry".to_owned()], digest));
    }

    /// Canned responses, so the challenge/token/retry flow can be driven
    /// through every branch without anyone's registry being involved.
    struct FakeRegistry {
        responses: std::sync::Mutex<Vec<ManifestProbe>>,
        token: Result<String, ConnectorError>,
        requests: std::sync::Mutex<Vec<(String, bool)>>,
        token_urls: std::sync::Mutex<Vec<String>>,
    }

    impl FakeRegistry {
        fn new(responses: Vec<ManifestProbe>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
                token: Ok("issued-token".to_owned()),
                requests: std::sync::Mutex::new(Vec::new()),
                token_urls: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl RegistryTransport for FakeRegistry {
        async fn head_manifest(
            &self,
            url: &str,
            token: Option<&str>,
        ) -> Result<ManifestProbe, ConnectorError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_owned(), token.is_some()));
            let mut responses = self.responses.lock().unwrap();
            Ok(responses.remove(0))
        }

        async fn fetch_token(&self, url: &str) -> Result<String, ConnectorError> {
            self.token_urls.lock().unwrap().push(url.to_owned());
            self.token.clone()
        }
    }

    fn probe(status: u16, digest: Option<&str>, challenge: Option<&str>) -> ManifestProbe {
        ManifestProbe {
            status,
            digest: digest.map(str::to_owned),
            challenge: challenge.map(str::to_owned),
        }
    }

    #[tokio::test]
    async fn a_registry_needing_no_token_answers_in_one_request() {
        let registry = FakeRegistry::new(vec![probe(200, Some("sha256:aaaa"), None)]);
        let reference = ImageReference::parse("registry.example.com/team/app:1").unwrap();

        assert_eq!(
            current_digest(&registry, &reference).await.unwrap(),
            "sha256:aaaa"
        );
        assert_eq!(
            *registry.requests.lock().unwrap(),
            vec![(
                "https://registry.example.com/v2/team/app/manifests/1".to_owned(),
                false
            )],
            "a registry that does not challenge must not be sent a token"
        );
        assert!(registry.token_urls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_challenge_is_followed_exactly_once() {
        let registry = FakeRegistry::new(vec![
            probe(
                401,
                None,
                Some(
                    r#"Bearer realm="https://auth.example.com/token",service="registry.example.com",scope="repository:library/alpine:pull""#,
                ),
            ),
            probe(200, Some("sha256:bbbb"), None),
        ]);
        let reference = ImageReference::parse("alpine:3.20").unwrap();

        assert_eq!(
            current_digest(&registry, &reference).await.unwrap(),
            "sha256:bbbb"
        );
        let requests = registry.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].1, "the first attempt is anonymous");
        assert!(requests[1].1, "the retry carries the issued token");
        assert_eq!(
            *registry.token_urls.lock().unwrap(),
            vec![
                "https://auth.example.com/token?service=registry.example.com\
                 &scope=repository:library/alpine:pull"
                    .to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn a_second_refusal_is_reported_rather_than_retried_forever() {
        let registry = FakeRegistry::new(vec![
            probe(
                401,
                None,
                Some(r#"Bearer realm="https://auth.example.com/token""#),
            ),
            probe(401, None, None),
        ]);
        let reference = ImageReference::parse("owner/private:v1").unwrap();

        let error = current_digest(&registry, &reference)
            .await
            .expect_err("a repository the token does not cover must fail");
        assert!(matches!(error, ConnectorError::AuthFailed { .. }));
        assert!(error.to_string().contains("private repositories"));
        assert_eq!(registry.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_rate_limited_check_is_recognisable_as_one() {
        let registry = FakeRegistry::new(vec![probe(429, None, None)]);
        let reference = ImageReference::parse("nginx").unwrap();

        let error = current_digest(&registry, &reference)
            .await
            .expect_err("429 is a failure");
        assert!(error.to_string().contains("rate-limiting"), "{error}");
    }

    #[tokio::test]
    async fn a_success_without_a_digest_header_is_not_silently_treated_as_up_to_date() {
        let registry = FakeRegistry::new(vec![probe(200, None, None)]);
        let reference = ImageReference::parse("nginx").unwrap();

        assert!(matches!(
            current_digest(&registry, &reference).await,
            Err(ConnectorError::Internal(_))
        ));
    }
}
