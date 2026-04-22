use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::sync::Mutex;

use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};
use serde::{Deserialize, Serialize};

/// Schema version for cassette files. Bump on breaking changes.
pub const CASSETTE_SCHEMA_VERSION: &str = "1";

/// A recorded interaction: one request and its response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteEntry {
    pub request: GatewayRequest,
    pub response: CassetteResponse,
}

/// The response side of a cassette entry.
/// Wraps Ok/Err to handle both success and error recordings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CassetteResponse {
    Success(GatewayResponse),
    Error {
        kind: String,
        message: String,
        retry_after_ms: Option<u64>,
    },
}

impl CassetteResponse {
    /// Convert to the `Result` type expected by `ProviderAdapter::complete`.
    ///
    /// Maps the `kind` string to the correct [`AdapterError`] variant:
    /// `"rate_limit"`, `"auth_error"`, `"timeout"`, `"provider_error"`, `"parse_error"`.
    pub fn to_adapter_result(self) -> Result<GatewayResponse, AdapterError> {
        match self {
            Self::Success(r) => Ok(r),
            Self::Error {
                kind,
                message,
                retry_after_ms,
            } => {
                let retry_after = retry_after_ms.map(std::time::Duration::from_millis);
                Err(match kind.as_str() {
                    "rate_limit" => AdapterError::RateLimit { retry_after },
                    "auth_error" => AdapterError::AuthError(message),
                    "timeout" => AdapterError::Timeout,
                    "provider_error" => AdapterError::ProviderError(message),
                    "parse_error" => AdapterError::ParseError(message),
                    other => AdapterError::ProviderError(format!(
                        "unknown cassette error kind '{other}': {message}"
                    )),
                })
            }
        }
    }
}

/// A complete cassette file — metadata + ordered entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cassette {
    pub schema_version: String,
    pub provider: String,
    /// ISO 8601 timestamp of when this cassette was recorded.
    pub recorded_at: String,
    pub entries: Vec<CassetteEntry>,
}

impl Cassette {
    pub fn from_file(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        serde_json::from_reader(file).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    pub fn to_file(&self, path: &Path) -> io::Result<()> {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Adapter that replays responses from a cassette file.
///
/// Drop-in replacement for [`RecordedAdapter`] but loaded from disk fixtures.
/// The adapter presents itself under the name passed to the constructor — use `"mock"` to
/// route through `GatewayMode::Mock` in integration tests.
///
/// [`RecordedAdapter`]: crate::adapters::recorded::RecordedAdapter
pub struct FixtureAdapter {
    adapter_name: String,
    entries: Mutex<VecDeque<CassetteEntry>>,
}

impl FixtureAdapter {
    /// Load from an in-memory [`Cassette`]. The adapter presents itself under `name`.
    pub fn from_cassette(name: impl Into<String>, cassette: Cassette) -> Self {
        Self {
            adapter_name: name.into(),
            entries: Mutex::new(VecDeque::from(cassette.entries)),
        }
    }

    /// Load from a cassette file on disk. The adapter presents itself under `name`.
    pub fn from_file(name: impl Into<String>, path: &Path) -> io::Result<Self> {
        let cassette = Cassette::from_file(path)?;
        Ok(Self::from_cassette(name, cassette))
    }
}

impl ProviderAdapter for FixtureAdapter {
    fn name(&self) -> &str {
        &self.adapter_name
    }

    fn complete(&self, _request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        let mut queue = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match queue.pop_front() {
            Some(entry) => entry.response.to_adapter_result(),
            None => Err(AdapterError::ProviderError(
                "cassette exhausted — no more recorded responses".to_string(),
            )),
        }
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Wraps a real [`ProviderAdapter`], forwards all calls, captures request/response pairs.
///
/// Call [`into_cassette`](RecordingAdapter::into_cassette) when done to produce a
/// [`Cassette`] suitable for writing to disk with [`Cassette::to_file`].
///
/// # Feature gate
///
/// Only compiled when `--features record-fixtures` is enabled.
/// Never use in CI or production builds — this wraps live providers and performs file I/O.
#[cfg(feature = "record-fixtures")]
pub struct RecordingAdapter<A: ProviderAdapter> {
    inner: A,
    provider_name: String,
    entries: Mutex<Vec<CassetteEntry>>,
}

#[cfg(feature = "record-fixtures")]
impl<A: ProviderAdapter> RecordingAdapter<A> {
    pub fn new(inner: A) -> Self {
        let provider_name = inner.name().to_string();
        Self {
            inner,
            provider_name,
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Consume the adapter and return a [`Cassette`] with all recorded entries.
    pub fn into_cassette(self) -> Cassette {
        let entries = self.entries.into_inner().unwrap_or_else(|e| e.into_inner());
        Cassette {
            schema_version: CASSETTE_SCHEMA_VERSION.to_string(),
            provider: self.provider_name,
            recorded_at: "2026-04-22T00:00:00Z".to_string(),
            entries,
        }
    }

    /// Write a snapshot of current entries to disk without consuming self.
    pub fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let entries = {
            let guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let cassette = Cassette {
            schema_version: CASSETTE_SCHEMA_VERSION.to_string(),
            provider: self.provider_name.clone(),
            recorded_at: "2026-04-22T00:00:00Z".to_string(),
            entries,
        };
        cassette.to_file(path)
    }
}

#[cfg(feature = "record-fixtures")]
impl<A: ProviderAdapter> ProviderAdapter for RecordingAdapter<A> {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        let result = self.inner.complete(request);
        let cassette_response = match &result {
            Ok(r) => CassetteResponse::Success(r.clone()),
            Err(e) => match e {
                AdapterError::RateLimit { retry_after } => CassetteResponse::Error {
                    kind: "rate_limit".to_string(),
                    message: "rate limited by provider".to_string(),
                    retry_after_ms: retry_after.map(|d| d.as_millis() as u64),
                },
                AdapterError::AuthError(msg) => CassetteResponse::Error {
                    kind: "auth_error".to_string(),
                    message: msg.clone(),
                    retry_after_ms: None,
                },
                AdapterError::Timeout => CassetteResponse::Error {
                    kind: "timeout".to_string(),
                    message: "provider request timed out".to_string(),
                    retry_after_ms: None,
                },
                AdapterError::ProviderError(msg) => CassetteResponse::Error {
                    kind: "provider_error".to_string(),
                    message: msg.clone(),
                    retry_after_ms: None,
                },
                AdapterError::ParseError(msg) => CassetteResponse::Error {
                    kind: "parse_error".to_string(),
                    message: msg.clone(),
                    retry_after_ms: None,
                },
            },
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(CassetteEntry {
            request: request.clone(),
            response: cassette_response,
        });
        result
    }

    fn is_available(&self) -> bool {
        self.inner.is_available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::{Message, ModelTier, RoutingConstraints};

    fn make_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Quick,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "Hello".to_string(),
            }],
            max_tokens: None,
            stream: false,
            metadata: Default::default(),
        }
    }

    fn make_response(content: &str) -> GatewayResponse {
        GatewayResponse {
            id: "fixture-1".to_string(),
            model: "gemini-2.0-flash".to_string(),
            content: content.to_string(),
            tokens_in: 5,
            tokens_out: 12,
            finish_reason: "stop".to_string(),
        }
    }

    fn make_cassette(entries: Vec<CassetteEntry>) -> Cassette {
        Cassette {
            schema_version: CASSETTE_SCHEMA_VERSION.to_string(),
            provider: "test".to_string(),
            recorded_at: "2026-04-22T00:00:00Z".to_string(),
            entries,
        }
    }

    // --- CassetteResponse round-trip serialization ---

    #[test]
    fn cassette_response_success_round_trips() {
        let response = make_response("hello world");
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Success(response),
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert_eq!(result.unwrap().content, "hello world");
    }

    #[test]
    fn cassette_response_rate_limit_round_trips() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Error {
                kind: "rate_limit".to_string(),
                message: "rate limited".to_string(),
                retry_after_ms: Some(30_000),
            },
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert!(matches!(
            result,
            Err(AdapterError::RateLimit {
                retry_after: Some(_)
            })
        ));
    }

    #[test]
    fn cassette_response_auth_error_round_trips() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Error {
                kind: "auth_error".to_string(),
                message: "invalid api key".to_string(),
                retry_after_ms: None,
            },
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert!(matches!(result, Err(AdapterError::AuthError(_))));
    }

    #[test]
    fn cassette_response_timeout_round_trips() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Error {
                kind: "timeout".to_string(),
                message: "timed out".to_string(),
                retry_after_ms: None,
            },
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert!(matches!(result, Err(AdapterError::Timeout)));
    }

    #[test]
    fn cassette_response_provider_error_round_trips() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Error {
                kind: "provider_error".to_string(),
                message: "internal server error".to_string(),
                retry_after_ms: None,
            },
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert!(matches!(result, Err(AdapterError::ProviderError(_))));
    }

    #[test]
    fn cassette_response_parse_error_round_trips() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Error {
                kind: "parse_error".to_string(),
                message: "unexpected format".to_string(),
                retry_after_ms: None,
            },
        }]);
        let json = serde_json::to_string(&cassette).unwrap();
        let deserialized: Cassette = serde_json::from_str(&json).unwrap();
        let result = deserialized
            .entries
            .into_iter()
            .next()
            .unwrap()
            .response
            .to_adapter_result();
        assert!(matches!(result, Err(AdapterError::ParseError(_))));
    }

    // --- FixtureAdapter ---

    #[test]
    fn fixture_adapter_replays_in_order() {
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("first")),
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("second")),
            },
        ]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_request();
        assert_eq!(adapter.complete(&req).unwrap().content, "first");
        assert_eq!(adapter.complete(&req).unwrap().content, "second");
    }

    #[test]
    fn fixture_adapter_exhaustion_returns_provider_error() {
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Success(make_response("one")),
        }]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_request();
        assert!(adapter.complete(&req).is_ok());
        let err = adapter.complete(&req).unwrap_err();
        assert!(matches!(err, AdapterError::ProviderError(_)));
    }

    #[test]
    fn fixture_adapter_is_always_available() {
        let adapter = FixtureAdapter::from_cassette("test", make_cassette(vec![]));
        assert!(adapter.is_available());
        assert_eq!(adapter.name(), "test");
    }

    // --- Cassette file round-trip ---

    #[test]
    fn cassette_file_round_trip() {
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("hello")),
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Error {
                    kind: "rate_limit".to_string(),
                    message: "rate limited".to_string(),
                    retry_after_ms: Some(5_000),
                },
            },
        ]);

        let path = std::env::temp_dir().join("cassette_round_trip_test.json");
        cassette.to_file(&path).unwrap();
        let loaded = Cassette::from_file(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(loaded.schema_version, CASSETTE_SCHEMA_VERSION);
        assert_eq!(loaded.provider, "test");
        assert_eq!(loaded.entries.len(), 2);
        // First entry round-trips as success
        let first = loaded.entries[0].response.clone().to_adapter_result();
        assert_eq!(first.unwrap().content, "hello");
        // Second entry round-trips as rate-limit error
        let second = loaded.entries[1].response.clone().to_adapter_result();
        assert!(matches!(
            second,
            Err(AdapterError::RateLimit {
                retry_after: Some(_)
            })
        ));
    }
}
