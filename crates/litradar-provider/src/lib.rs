//! Composable provider capabilities for indexing and live article access.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::sync::Arc;

use litradar_domain::{
    ArticleAccessContext, ArticleFullTextResolution, ArticleLocator, ArticleRedirect,
    JournalCatalogEntry, ProviderBatch, ProviderCapabilityKind,
};

pub mod conformance;

/// Provider operation failure category used by runtime fallback policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// The requested article is not available from this provider.
    NotFound,
    /// The provider requires an authenticated user session.
    AuthenticationRequired,
    /// The provider is temporarily unavailable and another provider may be tried.
    TemporarilyUnavailable,
    /// The provider returned a response that violated its declared contract.
    InvalidResponse,
    /// The provider could not complete the request because of an internal failure.
    Internal,
}

/// Safe provider operation error without raw upstream content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    kind: ProviderErrorKind,
    safe_message: String,
}

impl ProviderError {
    /// Build a safe provider error.
    ///
    /// # Arguments
    ///
    /// * `kind` - Stable provider failure category.
    /// * `safe_message` - Bounded diagnostic without URLs, credentials, or payload content.
    ///
    /// # Returns
    ///
    /// Provider error.
    pub fn new(kind: ProviderErrorKind, safe_message: impl Into<String>) -> Self {
        Self {
            kind,
            safe_message: safe_message.into(),
        }
    }

    /// Return the stable failure category.
    ///
    /// # Returns
    ///
    /// Provider failure kind.
    pub fn kind(&self) -> ProviderErrorKind {
        self.kind
    }
}

impl fmt::Display for ProviderError {
    /// Format the safe provider diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.safe_message)
    }
}

impl Error for ProviderError {}

/// Provider capability that returns canonical journal, issue, and article content.
pub trait IndexContentProvider: Send + Sync {
    /// Fetch one canonical page for a maintained journal.
    ///
    /// # Arguments
    ///
    /// * `catalog` - Provider-free maintained journal entry.
    /// * `checkpoint` - Opaque provider checkpoint from the control database.
    ///
    /// # Returns
    ///
    /// Canonical content batch or a safe provider error.
    fn fetch(
        &self,
        catalog: &JournalCatalogEntry,
        checkpoint: Option<&str>,
    ) -> Result<ProviderBatch, ProviderError>;
}

/// Optional provider capability for live article detail-page resolution.
pub trait ArticleDetailProvider: Send + Sync {
    /// Resolve an ephemeral article detail destination.
    ///
    /// # Arguments
    ///
    /// * `article` - Provider-neutral article locator.
    /// * `context` - Request authentication context.
    ///
    /// # Returns
    ///
    /// Ephemeral redirect or a safe provider error.
    fn resolve_detail(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError>;
}

/// Optional provider capability for live article abstract-page resolution.
pub trait ArticleAbstractProvider: Send + Sync {
    /// Resolve an ephemeral article abstract-page destination.
    ///
    /// # Arguments
    ///
    /// * `article` - Provider-neutral article locator.
    /// * `context` - Request authentication context.
    ///
    /// # Returns
    ///
    /// Ephemeral redirect or a safe provider error.
    fn resolve_abstract(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleRedirect, ProviderError>;
}

/// Optional provider capability for live article full-text resolution.
pub trait ArticleFullTextProvider: Send + Sync {
    /// Resolve an ephemeral full-text redirect or bounded document.
    ///
    /// # Arguments
    ///
    /// * `article` - Provider-neutral article locator.
    /// * `context` - Request authentication context.
    ///
    /// # Returns
    ///
    /// Full-text resolution or a safe provider error.
    fn resolve_full_text(
        &self,
        article: &ArticleLocator,
        context: ArticleAccessContext,
    ) -> Result<ArticleFullTextResolution, ProviderError>;
}

/// Capability declaration attached to a provider descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderCapabilities {
    /// Whether canonical indexing is implemented.
    pub index_content: bool,
    /// Whether live detail-page resolution is implemented.
    pub article_detail: bool,
    /// Whether live abstract-page resolution is implemented.
    pub article_abstract: bool,
    /// Whether live full-text resolution is implemented.
    pub article_full_text: bool,
}

impl ProviderCapabilities {
    /// Return whether this declaration contains one capability.
    ///
    /// # Arguments
    ///
    /// * `kind` - Capability to inspect.
    ///
    /// # Returns
    ///
    /// True when the provider declares the capability.
    pub fn contains(self, kind: ProviderCapabilityKind) -> bool {
        match kind {
            ProviderCapabilityKind::IndexContent => self.index_content,
            ProviderCapabilityKind::ArticleDetail => self.article_detail,
            ProviderCapabilityKind::ArticleAbstract => self.article_abstract,
            ProviderCapabilityKind::ArticleFullText => self.article_full_text,
        }
    }

    fn is_empty(self) -> bool {
        !self.index_content
            && !self.article_detail
            && !self.article_abstract
            && !self.article_full_text
    }
}

/// Stable provider name and declared capabilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    /// Stable lowercase runtime provider name.
    pub name: String,
    /// Explicit capability declaration.
    pub capabilities: ProviderCapabilities,
}

/// Concrete optional provider implementations supplied during registration.
#[derive(Default)]
pub struct ProviderImplementations {
    /// Canonical indexing implementation.
    pub index_content: Option<Arc<dyn IndexContentProvider>>,
    /// Live detail-page implementation.
    pub article_detail: Option<Arc<dyn ArticleDetailProvider>>,
    /// Live abstract-page implementation.
    pub article_abstract: Option<Arc<dyn ArticleAbstractProvider>>,
    /// Live full-text implementation.
    pub article_full_text: Option<Arc<dyn ArticleFullTextProvider>>,
}

impl ProviderImplementations {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            index_content: self.index_content.is_some(),
            article_detail: self.article_detail.is_some(),
            article_abstract: self.article_abstract.is_some(),
            article_full_text: self.article_full_text.is_some(),
        }
    }
}

/// Provider registration with implementations matching its declaration exactly.
pub struct ProviderRegistration {
    descriptor: ProviderDescriptor,
    implementations: ProviderImplementations,
}

impl ProviderRegistration {
    /// Validate and build one provider registration.
    ///
    /// # Arguments
    ///
    /// * `descriptor` - Stable name and explicit capability declaration.
    /// * `implementations` - Optional capability implementations.
    ///
    /// # Returns
    ///
    /// Validated registration or a registry error.
    pub fn try_new(
        descriptor: ProviderDescriptor,
        implementations: ProviderImplementations,
    ) -> Result<Self, ProviderRegistryError> {
        validate_provider_name(&descriptor.name)?;
        if descriptor.capabilities.is_empty() {
            return Err(ProviderRegistryError::NoCapabilities(
                descriptor.name.clone(),
            ));
        }
        if descriptor.capabilities != implementations.capabilities() {
            return Err(ProviderRegistryError::CapabilityMismatch(
                descriptor.name.clone(),
            ));
        }
        Ok(Self {
            descriptor,
            implementations,
        })
    }

    /// Return the immutable provider descriptor.
    ///
    /// # Returns
    ///
    /// Provider descriptor.
    pub fn descriptor(&self) -> &ProviderDescriptor {
        &self.descriptor
    }

    /// Return the indexing implementation when declared.
    ///
    /// # Returns
    ///
    /// Optional indexing provider.
    pub fn index_content(&self) -> Option<&Arc<dyn IndexContentProvider>> {
        self.implementations.index_content.as_ref()
    }

    /// Return the detail-page implementation when declared.
    ///
    /// # Returns
    ///
    /// Optional detail provider.
    pub fn article_detail(&self) -> Option<&Arc<dyn ArticleDetailProvider>> {
        self.implementations.article_detail.as_ref()
    }

    /// Return the abstract-page implementation when declared.
    ///
    /// # Returns
    ///
    /// Optional abstract provider.
    pub fn article_abstract(&self) -> Option<&Arc<dyn ArticleAbstractProvider>> {
        self.implementations.article_abstract.as_ref()
    }

    /// Return the full-text implementation when declared.
    ///
    /// # Returns
    ///
    /// Optional full-text provider.
    pub fn article_full_text(&self) -> Option<&Arc<dyn ArticleFullTextProvider>> {
        self.implementations.article_full_text.as_ref()
    }
}

/// Provider registry construction failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRegistryError {
    /// Provider name does not satisfy the stable runtime format.
    InvalidName(String),
    /// Provider declares no usable capability.
    NoCapabilities(String),
    /// Descriptor claims do not match supplied implementations.
    CapabilityMismatch(String),
    /// A provider name was registered more than once.
    DuplicateName(String),
}

impl fmt::Display for ProviderRegistryError {
    /// Format the registry failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(formatter, "invalid provider name: {name}"),
            Self::NoCapabilities(name) => {
                write!(formatter, "provider declares no capabilities: {name}")
            }
            Self::CapabilityMismatch(name) => {
                write!(
                    formatter,
                    "provider capability declaration mismatch: {name}"
                )
            }
            Self::DuplicateName(name) => write!(formatter, "duplicate provider name: {name}"),
        }
    }
}

impl Error for ProviderRegistryError {}

/// Deterministic registry of provider capability implementations.
#[derive(Default)]
pub struct ProviderRegistry {
    registrations: BTreeMap<String, ProviderRegistration>,
}

impl ProviderRegistry {
    /// Insert one validated provider registration.
    ///
    /// # Arguments
    ///
    /// * `registration` - Provider registration to insert.
    ///
    /// # Returns
    ///
    /// Success or a duplicate-name error.
    pub fn register(
        &mut self,
        registration: ProviderRegistration,
    ) -> Result<(), ProviderRegistryError> {
        let name = registration.descriptor.name.clone();
        if self.registrations.contains_key(&name) {
            return Err(ProviderRegistryError::DuplicateName(name));
        }
        self.registrations.insert(name, registration);
        Ok(())
    }

    /// Find one provider by stable runtime name.
    ///
    /// # Arguments
    ///
    /// * `name` - Stable provider name.
    ///
    /// # Returns
    ///
    /// Matching provider registration when present.
    pub fn find(&self, name: &str) -> Option<&ProviderRegistration> {
        self.registrations.get(name)
    }

    /// Return registered providers that implement one capability in name order.
    ///
    /// # Arguments
    ///
    /// * `kind` - Capability to filter by.
    ///
    /// # Returns
    ///
    /// Deterministically ordered provider registrations.
    pub fn providers_with(&self, kind: ProviderCapabilityKind) -> Vec<&ProviderRegistration> {
        self.registrations
            .values()
            .filter(|registration| registration.descriptor.capabilities.contains(kind))
            .collect()
    }
}

fn validate_provider_name(name: &str) -> Result<(), ProviderRegistryError> {
    if !(2..=64).contains(&name.len())
        || !name.is_ascii()
        || !name.bytes().enumerate().all(|(index, byte)| match byte {
            b'a'..=b'z' | b'0'..=b'9' => true,
            b'_' | b'-' => index > 0,
            _ => false,
        })
    {
        return Err(ProviderRegistryError::InvalidName(name.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use litradar_domain::{
        ArticleAccessContext, ArticleFullTextResolution, ArticleLocator, ArticleRedirect,
        JournalCatalogEntry, ProviderBatch, ProviderCapabilityKind,
    };

    use super::{
        ArticleAbstractProvider, ArticleDetailProvider, ArticleFullTextProvider,
        IndexContentProvider, ProviderCapabilities, ProviderDescriptor, ProviderError,
        ProviderImplementations, ProviderRegistration, ProviderRegistry, ProviderRegistryError,
    };

    struct FakeProvider;

    impl IndexContentProvider for FakeProvider {
        fn fetch(
            &self,
            _catalog: &JournalCatalogEntry,
            _checkpoint: Option<&str>,
        ) -> Result<ProviderBatch, ProviderError> {
            unreachable!("registration tests do not fetch")
        }
    }

    impl ArticleDetailProvider for FakeProvider {
        fn resolve_detail(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            unreachable!("registration tests do not resolve")
        }
    }

    impl ArticleAbstractProvider for FakeProvider {
        fn resolve_abstract(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleRedirect, ProviderError> {
            unreachable!("registration tests do not resolve")
        }
    }

    impl ArticleFullTextProvider for FakeProvider {
        fn resolve_full_text(
            &self,
            _article: &ArticleLocator,
            _context: ArticleAccessContext,
        ) -> Result<ArticleFullTextResolution, ProviderError> {
            unreachable!("registration tests do not resolve")
        }
    }

    #[test]
    fn accepts_every_partial_capability_shape() {
        let provider = Arc::new(FakeProvider);
        let cases = [
            (
                "index-only",
                ProviderCapabilities {
                    index_content: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    index_content: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "detail-only",
                ProviderCapabilities {
                    article_detail: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    article_detail: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "abstract-only",
                ProviderCapabilities {
                    article_abstract: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    article_abstract: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "fulltext-only",
                ProviderCapabilities {
                    article_full_text: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    article_full_text: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "two-online",
                ProviderCapabilities {
                    article_detail: true,
                    article_abstract: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    article_detail: Some(provider.clone()),
                    article_abstract: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "three-online",
                ProviderCapabilities {
                    article_detail: true,
                    article_abstract: true,
                    article_full_text: true,
                    ..ProviderCapabilities::default()
                },
                ProviderImplementations {
                    article_detail: Some(provider.clone()),
                    article_abstract: Some(provider.clone()),
                    article_full_text: Some(provider.clone()),
                    ..ProviderImplementations::default()
                },
            ),
            (
                "all-capabilities",
                ProviderCapabilities {
                    index_content: true,
                    article_detail: true,
                    article_abstract: true,
                    article_full_text: true,
                },
                ProviderImplementations {
                    index_content: Some(provider.clone()),
                    article_detail: Some(provider.clone()),
                    article_abstract: Some(provider.clone()),
                    article_full_text: Some(provider.clone()),
                },
            ),
        ];

        for (name, capabilities, implementations) in cases {
            ProviderRegistration::try_new(
                ProviderDescriptor {
                    name: name.to_string(),
                    capabilities,
                },
                implementations,
            )
            .expect("partial capability registration should pass");
        }
    }

    #[test]
    fn rejects_false_capability_advertising() {
        let error = ProviderRegistration::try_new(
            ProviderDescriptor {
                name: "false-provider".to_string(),
                capabilities: ProviderCapabilities {
                    article_detail: true,
                    ..ProviderCapabilities::default()
                },
            },
            ProviderImplementations::default(),
        )
        .err()
        .expect("missing implementation should fail");
        assert_eq!(
            error,
            ProviderRegistryError::CapabilityMismatch("false-provider".to_string())
        );

        let error = ProviderRegistration::try_new(
            ProviderDescriptor {
                name: "empty-provider".to_string(),
                capabilities: ProviderCapabilities::default(),
            },
            ProviderImplementations::default(),
        )
        .err()
        .expect("empty provider should fail");
        assert_eq!(
            error,
            ProviderRegistryError::NoCapabilities("empty-provider".to_string())
        );
    }

    #[test]
    fn registry_filters_capabilities_and_rejects_duplicates() {
        let provider = Arc::new(FakeProvider);
        let registration = ProviderRegistration::try_new(
            ProviderDescriptor {
                name: "mixed-provider".to_string(),
                capabilities: ProviderCapabilities {
                    index_content: true,
                    article_full_text: true,
                    ..ProviderCapabilities::default()
                },
            },
            ProviderImplementations {
                index_content: Some(provider.clone()),
                article_full_text: Some(provider),
                ..ProviderImplementations::default()
            },
        )
        .expect("mixed registration should pass");
        let mut registry = ProviderRegistry::default();
        registry.register(registration).expect("register provider");

        assert_eq!(
            registry
                .providers_with(ProviderCapabilityKind::IndexContent)
                .len(),
            1
        );
        assert!(registry
            .providers_with(ProviderCapabilityKind::ArticleDetail)
            .is_empty());

        let duplicate = ProviderRegistration::try_new(
            ProviderDescriptor {
                name: "mixed-provider".to_string(),
                capabilities: ProviderCapabilities {
                    index_content: true,
                    ..ProviderCapabilities::default()
                },
            },
            ProviderImplementations {
                index_content: Some(Arc::new(FakeProvider)),
                ..ProviderImplementations::default()
            },
        )
        .expect("duplicate registration fixture should pass");
        assert_eq!(
            registry
                .register(duplicate)
                .expect_err("duplicate should fail"),
            ProviderRegistryError::DuplicateName("mixed-provider".to_string())
        );
    }
}
