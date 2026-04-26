use std::io;
use std::path::Path;
use std::sync::Mutex;

use accelmars_gateway_core::{AdapterError, GatewayRequest, GatewayResponse, ProviderAdapter};
use serde::{Deserialize, Serialize};

/// Schema version for cassette files. Bump on breaking changes.
pub const CASSETTE_SCHEMA_VERSION: &str = "1";

/// Optional content-based match criteria for a cassette entry.
///
/// All fields are optional — a field set to `None` matches any value for that dimension.
/// An `EntryMatcher` with all fields `None` is valid but unusual: it matches any request,
/// behaving identically to sequential mode.
/// A cassette entry without a `match_key` (`None`) always falls through to sequential replay.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EntryMatcher {
    /// Match if request tier equals this value. Case-insensitive.
    /// Valid values: `"quick"` | `"standard"` | `"max"` | `"ultra"`.
    /// A mismatched tier produces no match (not an error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// Match if the last message in the request contains this substring.
    /// Case-insensitive. Matches on `request.messages.last().content`.
    /// If the request has no messages, this criterion is not satisfied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_contains: Option<String>,

    /// Match if the total message count equals this value.
    /// Counts all messages in `request.messages`, including system messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<usize>,
}

impl EntryMatcher {
    /// Returns `true` if all non-`None` fields in this matcher satisfy `request`.
    ///
    /// Evaluation is AND-logic: every non-`None` field must match for the overall
    /// result to be `true`. A matcher with all fields `None` always returns `true`.
    pub fn matches(&self, request: &GatewayRequest) -> bool {
        if let Some(tier) = &self.tier {
            if request.tier.to_string() != tier.to_lowercase() {
                return false;
            }
        }
        if let Some(contains) = &self.last_message_contains {
            let last_content = request
                .messages
                .last()
                .map(|m| m.content.to_lowercase())
                .unwrap_or_default();
            if !last_content.contains(&contains.to_lowercase()) {
                return false;
            }
        }
        if let Some(count) = self.message_count {
            if request.messages.len() != count {
                return false;
            }
        }
        true
    }
}

/// A recorded interaction: one request and its response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteEntry {
    pub request: GatewayRequest,
    pub response: CassetteResponse,
    /// Optional match criteria. `None` means sequential replay (existing behavior).
    ///
    /// When `Some`, the entry participates in keyed lookup: the adapter scans all
    /// entries with `match_key: Some(m)` and returns the first one where `m.matches(request)`.
    /// Only if no keyed entry matches does the adapter fall back to sequential replay
    /// (the first entry with `match_key: None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_key: Option<EntryMatcher>,
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
    entries: Mutex<Vec<CassetteEntry>>,
}

impl FixtureAdapter {
    /// Load from an in-memory [`Cassette`]. The adapter presents itself under `name`.
    pub fn from_cassette(name: impl Into<String>, cassette: Cassette) -> Self {
        Self {
            adapter_name: name.into(),
            entries: Mutex::new(cassette.entries),
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

    fn complete(&self, request: &GatewayRequest) -> Result<GatewayResponse, AdapterError> {
        let maybe_response = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            // Keyed lookup first; sequential fallback if no keyed entry matches.
            let keyed_idx = entries
                .iter()
                .position(|e| e.match_key.as_ref().is_some_and(|m| m.matches(request)));
            let idx = keyed_idx.or_else(|| entries.iter().position(|e| e.match_key.is_none()));
            idx.map(|i| entries.remove(i).response)
        }; // lock drops here
        match maybe_response {
            Some(response) => response.to_adapter_result(),
            None => Err(AdapterError::ProviderError(
                "cassette exhausted — no matching entry".to_string(),
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
            match_key: None,
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

    fn make_max_request() -> GatewayRequest {
        GatewayRequest {
            tier: ModelTier::Max,
            constraints: RoutingConstraints::default(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "synthesize decision".to_string(),
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
            match_key: None,
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
            match_key: None,
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
            match_key: None,
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
            match_key: None,
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
            match_key: None,
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
            match_key: None,
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
                match_key: None,
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("second")),
                match_key: None,
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
            match_key: None,
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
                match_key: None,
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Error {
                    kind: "rate_limit".to_string(),
                    message: "rate limited".to_string(),
                    retry_after_ms: Some(5_000),
                },
                match_key: None,
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

    // --- Keyed matching ---

    #[test]
    fn keyed_entry_matched_before_sequential_regardless_of_position() {
        // Sequential entry is at index 0, keyed entry (tier=max) is at index 1.
        // A max-tier request should consume the keyed entry, skipping index 0.
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("sequential")),
                match_key: None,
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("keyed")),
                match_key: Some(EntryMatcher {
                    tier: Some("max".to_string()),
                    ..Default::default()
                }),
            },
        ]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_max_request();
        // Keyed entry consumed first (position 1 skips ahead of position 0)
        assert_eq!(adapter.complete(&req).unwrap().content, "keyed");
        // Sequential entry still present
        assert_eq!(adapter.complete(&req).unwrap().content, "sequential");
    }

    #[test]
    fn keyed_fallback_to_sequential_when_no_match() {
        // Keyed entry requires standard tier; request is max — no match.
        // Adapter falls back to the sequential entry.
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("keyed-standard")),
                match_key: Some(EntryMatcher {
                    tier: Some("standard".to_string()),
                    ..Default::default()
                }),
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("sequential")),
                match_key: None,
            },
        ]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_max_request();
        // Max request does not match standard keyed entry → sequential fallback
        assert_eq!(adapter.complete(&req).unwrap().content, "sequential");
    }

    #[test]
    fn keyed_exhausted_returns_matching_error() {
        // Only a keyed entry (standard tier); no sequential fallback.
        // A max-tier request matches nothing → specific error message.
        let cassette = make_cassette(vec![CassetteEntry {
            request: make_request(),
            response: CassetteResponse::Success(make_response("keyed-standard")),
            match_key: Some(EntryMatcher {
                tier: Some("standard".to_string()),
                ..Default::default()
            }),
        }]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_max_request();
        let err = adapter.complete(&req).unwrap_err();
        assert!(
            matches!(&err, AdapterError::ProviderError(msg) if msg.contains("no matching entry"))
        );
    }

    #[test]
    fn all_none_matcher_is_keyed_not_sequential() {
        // EntryMatcher with all fields None is a keyed entry that matches any request.
        // It should be consumed ahead of the sequential entry (index 0).
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("sequential")),
                match_key: None,
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("all-none-keyed")),
                match_key: Some(EntryMatcher::default()),
            },
        ]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        let req = make_request();
        // All-None matcher is keyed and matches any request — consumed first
        assert_eq!(adapter.complete(&req).unwrap().content, "all-none-keyed");
        // Sequential entry still available
        assert_eq!(adapter.complete(&req).unwrap().content, "sequential");
    }

    #[test]
    fn mixed_cassette_consumed_in_correct_order() {
        // Cassette: [sequential, keyed(max), sequential].
        // max-tier request consumes the keyed entry. Remaining calls use sequential entries.
        let cassette = make_cassette(vec![
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("seq-1")),
                match_key: None,
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("max-keyed")),
                match_key: Some(EntryMatcher {
                    tier: Some("max".to_string()),
                    ..Default::default()
                }),
            },
            CassetteEntry {
                request: make_request(),
                response: CassetteResponse::Success(make_response("seq-2")),
                match_key: None,
            },
        ]);
        let adapter = FixtureAdapter::from_cassette("test", cassette);
        // Max-tier request consumes keyed entry at index 1
        assert_eq!(
            adapter.complete(&make_max_request()).unwrap().content,
            "max-keyed"
        );
        // Sequential entries consumed in order
        assert_eq!(adapter.complete(&make_request()).unwrap().content, "seq-1");
        assert_eq!(adapter.complete(&make_request()).unwrap().content, "seq-2");
        // Cassette exhausted
        assert!(adapter.complete(&make_request()).is_err());
    }
}
