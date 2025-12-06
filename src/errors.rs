//! Error types for the Polymarket client
//!
//! This module defines all error types used throughout the client, designed
//! for clear error handling in trading environments where fast error recovery
//! is critical.

use std::time::Duration;
use thiserror::Error;

/// Main error type for the Polymarket client
#[derive(Error, Debug)]
pub enum PolyfillError {
    /// Network-related errors (retryable)
    #[error("Network error: {message}")]
    Network {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// API errors from Polymarket
    #[error("API error ({status}): {message}")]
    Api {
        status: u16,
        message: String,
        error_code: Option<String>,
    },

    /// Authentication/authorization errors
    #[error("Auth error: {message}")]
    Auth {
        message: String,
        kind: AuthErrorKind,
    },

    /// Order-related errors
    #[error("Order error: {message}")]
    Order {
        message: String,
        kind: OrderErrorKind,
    },

    /// Market data errors
    #[error("Market data error: {message}")]
    MarketData {
        message: String,
        kind: MarketDataErrorKind,
    },

    /// Configuration errors
    #[error("Config error: {message}")]
    Config { message: String },

    /// Parsing/serialization errors
    #[error("Parse error: {message}")]
    Parse {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// Timeout errors
    #[error("Timeout error: operation timed out after {duration:?}")]
    Timeout {
        duration: Duration,
        operation: String,
    },

    /// Rate limiting errors
    #[error("Rate limit exceeded: {message}")]
    RateLimit {
        message: String,
        retry_after: Option<Duration>,
    },

    /// WebSocket/streaming errors
    #[error("Stream error: {message}")]
    Stream {
        message: String,
        kind: StreamErrorKind,
    },

    /// Validation errors
    #[error("Validation error: {message}")]
    Validation {
        message: String,
        field: Option<String>,
    },

    /// Internal errors (bugs)
    #[error("Internal error: {message}")]
    Internal {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

/// Authentication error subcategories
#[derive(Debug, Clone, PartialEq)]
pub enum AuthErrorKind {
    InvalidCredentials,
    ExpiredCredentials,
    InsufficientPermissions,
    SignatureError,
    NonceError,
}

/// Order error subcategories
#[derive(Debug, Clone, PartialEq)]
pub enum OrderErrorKind {
    InvalidPrice,
    InvalidSize,
    InsufficientBalance,
    MarketClosed,
    DuplicateOrder,
    OrderNotFound,
    CancellationFailed,
    ExecutionFailed,
    SizeConstraint,
    PriceConstraint,
}

/// Market data error subcategories
#[derive(Debug, Clone, PartialEq)]
pub enum MarketDataErrorKind {
    TokenNotFound,
    MarketNotFound,
    StaleData,
    IncompleteData,
    BookUnavailable,
}

/// Streaming error subcategories
#[derive(Debug, Clone, PartialEq)]
pub enum StreamErrorKind {
    ConnectionFailed,
    ConnectionLost,
    SubscriptionFailed,
    MessageCorrupted,
    Reconnecting,
}

impl PolyfillError {
    /// Check if this error is retryable
    pub fn is_retryable(&self) -> bool {
        match self {
            PolyfillError::Network { .. } => true,
            PolyfillError::Api { status, .. } => {
                // 5xx errors are typically retryable
                *status >= 500 && *status < 600
            },
            PolyfillError::Timeout { .. } => true,
            PolyfillError::RateLimit { .. } => true,
            PolyfillError::Stream { kind, .. } => {
                matches!(
                    kind,
                    StreamErrorKind::ConnectionLost | StreamErrorKind::Reconnecting
                )
            },
            _ => false,
        }
    }

    /// Get suggested retry delay
    pub fn retry_delay(&self) -> Option<Duration> {
        match self {
            PolyfillError::Network { .. } => Some(Duration::from_millis(100)),
            PolyfillError::Api { status, .. } => {
                if *status >= 500 {
                    Some(Duration::from_millis(500))
                } else {
                    None
                }
            },
            PolyfillError::Timeout { .. } => Some(Duration::from_millis(50)),
            PolyfillError::RateLimit { retry_after, .. } => {
                retry_after.or(Some(Duration::from_secs(1)))
            },
            PolyfillError::Stream { .. } => Some(Duration::from_millis(250)),
            _ => None,
        }
    }

    /// Check if this is a critical error that should stop trading
    pub fn is_critical(&self) -> bool {
        match self {
            PolyfillError::Auth { .. } => true,
            PolyfillError::Config { .. } => true,
            PolyfillError::Internal { .. } => true,
            PolyfillError::Order { kind, .. } => {
                matches!(kind, OrderErrorKind::InsufficientBalance)
            },
            _ => false,
        }
    }

    /// Get error category for metrics
    pub fn category(&self) -> &'static str {
        match self {
            PolyfillError::Network { .. } => "network",
            PolyfillError::Api { .. } => "api",
            PolyfillError::Auth { .. } => "auth",
            PolyfillError::Order { .. } => "order",
            PolyfillError::MarketData { .. } => "market_data",
            PolyfillError::Config { .. } => "config",
            PolyfillError::Parse { .. } => "parse",
            PolyfillError::Timeout { .. } => "timeout",
            PolyfillError::RateLimit { .. } => "rate_limit",
            PolyfillError::Stream { .. } => "stream",
            PolyfillError::Validation { .. } => "validation",
            PolyfillError::Internal { .. } => "internal",
        }
    }
}

// Convenience constructors
impl PolyfillError {
    pub fn network<E: std::error::Error + Send + Sync + 'static>(
        message: impl Into<String>,
        source: E,
    ) -> Self {
        Self::Network {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn api(status: u16, message: impl Into<String>) -> Self {
        Self::Api {
            status,
            message: message.into(),
            error_code: None,
        }
    }

    pub fn auth(message: impl Into<String>) -> Self {
        Self::Auth {
            message: message.into(),
            kind: AuthErrorKind::SignatureError,
        }
    }

    pub fn crypto(message: impl Into<String>) -> Self {
        Self::Auth {
            message: message.into(),
            kind: AuthErrorKind::SignatureError,
        }
    }

    pub fn order(message: impl Into<String>, kind: OrderErrorKind) -> Self {
        Self::Order {
            message: message.into(),
            kind,
        }
    }

    pub fn market_data(message: impl Into<String>, kind: MarketDataErrorKind) -> Self {
        Self::MarketData {
            message: message.into(),
            kind,
        }
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::Config {
            message: message.into(),
        }
    }

    pub fn parse(
        message: impl Into<String>,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self::Parse {
            message: message.into(),
            source,
        }
    }

    pub fn timeout(duration: Duration, operation: impl Into<String>) -> Self {
        Self::Timeout {
            duration,
            operation: operation.into(),
        }
    }

    pub fn rate_limit(message: impl Into<String>) -> Self {
        Self::RateLimit {
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn stream(message: impl Into<String>, kind: StreamErrorKind) -> Self {
        Self::Stream {
            message: message.into(),
            kind,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            field: None,
        }
    }

    pub fn internal<E: std::error::Error + Send + Sync + 'static>(
        message: impl Into<String>,
        source: E,
    ) -> Self {
        Self::Internal {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn internal_simple(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            source: None,
        }
    }
}

// Implement From for common external error types
impl From<reqwest::Error> for PolyfillError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            PolyfillError::Timeout {
                duration: Duration::from_secs(30), // default timeout
                operation: "HTTP request".to_string(),
            }
        } else if err.is_connect() || err.is_request() {
            PolyfillError::network("HTTP request failed", err)
        } else {
            PolyfillError::internal("Unexpected reqwest error", err)
        }
    }
}

impl From<serde_json::Error> for PolyfillError {
    fn from(err: serde_json::Error) -> Self {
        PolyfillError::Parse {
            message: format!("JSON parsing failed: {}", err),
            source: Some(Box::new(err)),
        }
    }
}

impl From<url::ParseError> for PolyfillError {
    fn from(err: url::ParseError) -> Self {
        PolyfillError::config(format!("Invalid URL: {}", err))
    }
}

#[cfg(feature = "stream")]
impl From<tokio_tungstenite::tungstenite::Error> for PolyfillError {
    fn from(err: tokio_tungstenite::tungstenite::Error) -> Self {
        use tokio_tungstenite::tungstenite::Error as WsError;

        let kind = match &err {
            WsError::ConnectionClosed | WsError::AlreadyClosed => StreamErrorKind::ConnectionLost,
            WsError::Io(_) => StreamErrorKind::ConnectionFailed,
            WsError::Protocol(_) => StreamErrorKind::MessageCorrupted,
            _ => StreamErrorKind::ConnectionFailed,
        };

        PolyfillError::stream(format!("WebSocket error: {}", err), kind)
    }
}

// Manual Clone implementation since Box<dyn Error> doesn't implement Clone
impl Clone for PolyfillError {
    fn clone(&self) -> Self {
        match self {
            PolyfillError::Network { message, source: _ } => PolyfillError::Network {
                message: message.clone(),
                source: None,
            },
            PolyfillError::Api {
                status,
                message,
                error_code,
            } => PolyfillError::Api {
                status: *status,
                message: message.clone(),
                error_code: error_code.clone(),
            },
            PolyfillError::Auth { message, kind } => PolyfillError::Auth {
                message: message.clone(),
                kind: kind.clone(),
            },
            PolyfillError::Order { message, kind } => PolyfillError::Order {
                message: message.clone(),
                kind: kind.clone(),
            },
            PolyfillError::MarketData { message, kind } => PolyfillError::MarketData {
                message: message.clone(),
                kind: kind.clone(),
            },
            PolyfillError::Config { message } => PolyfillError::Config {
                message: message.clone(),
            },
            PolyfillError::Parse { message, source: _ } => PolyfillError::Parse {
                message: message.clone(),
                source: None,
            },
            PolyfillError::Timeout {
                duration,
                operation,
            } => PolyfillError::Timeout {
                duration: *duration,
                operation: operation.clone(),
            },
            PolyfillError::RateLimit {
                message,
                retry_after,
            } => PolyfillError::RateLimit {
                message: message.clone(),
                retry_after: *retry_after,
            },
            PolyfillError::Stream { message, kind } => PolyfillError::Stream {
                message: message.clone(),
                kind: kind.clone(),
            },
            PolyfillError::Validation { message, field } => PolyfillError::Validation {
                message: message.clone(),
                field: field.clone(),
            },
            PolyfillError::Internal { message, source: _ } => PolyfillError::Internal {
                message: message.clone(),
                source: None,
            },
        }
    }
}

/// Result type alias for convenience
pub type Result<T> = std::result::Result<T, PolyfillError>;
