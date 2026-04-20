use std::collections::HashMap;
use std::sync::Arc;

use accelmars_gateway_core::ProviderAdapter;

/// Maps provider names to adapter instances.
///
/// Phase 1: simple name-based lookup + provider override.
/// PF-004 adds tier-based routing via config.
pub struct AdapterRegistry {
    adapters: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn ProviderAdapter>) {
        let name = adapter.name().to_string();
        tracing::info!(
            provider = %name,
            available = adapter.is_available(),
            "registered adapter"
        );
        self.adapters.insert(name, adapter);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.adapters.get(name).cloned()
    }

    /// Provider names where `is_available()` returns true.
    pub fn available(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .adapters
            .iter()
            .filter(|(_, a)| a.is_available())
            .map(|(name, _)| name.as_str())
            .collect();
        names.sort();
        names
    }

    /// All registered provider names (available or not).
    pub fn all_providers(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.adapters.keys().map(|k| k.as_str()).collect();
        names.sort();
        names
    }

    /// Resolve a provider adapter for a request.
    ///
    /// Priority:
    /// 1. Explicit provider override (from routing constraints)
    /// 2. First available non-mock adapter
    /// 3. Mock adapter (fallback)
    ///
    /// Full tier-based routing (tier → config-mapped provider) arrives in PF-004.
    pub fn resolve(&self, provider_override: Option<&str>) -> Option<Arc<dyn ProviderAdapter>> {
        // Explicit override
        if let Some(name) = provider_override {
            if let Some(adapter) = self.adapters.get(name) {
                if adapter.is_available() {
                    return Some(adapter.clone());
                }
            }
        }

        // First available non-mock adapter
        let mut available: Vec<_> = self
            .adapters
            .iter()
            .filter(|(name, a)| a.is_available() && name.as_str() != "mock")
            .collect();
        available.sort_by_key(|(name, _)| (*name).clone());

        if let Some((_, adapter)) = available.first() {
            return Some((*adapter).clone());
        }

        // Fall back to mock
        self.adapters.get("mock").cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accelmars_gateway_core::MockAdapter;

    #[test]
    fn register_and_get() {
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        assert!(registry.get("mock").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn available_returns_only_available_adapters() {
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        let available = registry.available();
        assert!(available.contains(&"mock"));
    }

    #[test]
    fn resolve_with_explicit_override() {
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        let adapter = registry.resolve(Some("mock")).unwrap();
        assert_eq!(adapter.name(), "mock");
    }

    #[test]
    fn resolve_falls_back_to_mock() {
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(MockAdapter::default()));
        // No non-mock adapters → falls back to mock
        let adapter = registry.resolve(None).unwrap();
        assert_eq!(adapter.name(), "mock");
    }

    #[test]
    fn resolve_returns_none_when_empty() {
        let registry = AdapterRegistry::new();
        assert!(registry.resolve(None).is_none());
    }
}
