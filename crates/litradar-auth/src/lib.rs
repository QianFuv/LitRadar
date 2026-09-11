//! Authentication compatibility helpers.

pub mod password;
pub mod service;
pub mod session;
pub mod token;

pub use litradar_domain::{
    ACCESS_TOKEN_ACTIVE_LIMIT, ACCESS_TOKEN_LIMIT_DETAIL, ACCESS_TOKEN_NAME_LENGTH_DETAIL,
    ACCESS_TOKEN_NAME_MAX_CODE_POINTS, ACCESS_TOKEN_RESERVED_NAME,
    ACCESS_TOKEN_RESERVED_NAME_DETAIL, ACCESS_TOKEN_TTL_DETAIL, ACCESS_TOKEN_TTL_MAX_SECONDS,
    ACCESS_TOKEN_TTL_MIN_SECONDS, ACCESS_TOKEN_VALIDATION_ORDER,
};

pub use password::{
    hash_legacy_password, hash_password, is_valid_new_password, verify_password, PasswordError,
    PasswordVerification, ARGON2_MEMORY_KIB, ARGON2_PARALLELISM, ARGON2_TIME_COST,
    MIN_PASSWORD_LENGTH, PBKDF2_ITERATIONS,
};
pub use service::{
    is_valid_username, AuthService, AuthServiceError, LoginSession, ACCESS_TOKEN_DEFAULT_TTL,
};
pub use session::SESSION_COOKIE_NAME;
pub use token::hash_token;
