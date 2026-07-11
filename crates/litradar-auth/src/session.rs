//! Browser session cookie compatibility constants.

/// Browser session cookie name used by the Python API.
pub const SESSION_COOKIE_NAME: &str = "litradar_session";
/// Session cookie path used by the Python API.
pub const SESSION_COOKIE_PATH: &str = "/";
/// SameSite setting used by the Python API.
pub const SESSION_COOKIE_SAME_SITE: &str = "lax";
/// Environment variable controlling the Secure cookie flag.
pub const AUTH_COOKIE_SECURE_ENV: &str = "AUTH_COOKIE_SECURE";

/// Browser session cookie policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCookiePolicy {
    /// Cookie name.
    pub name: &'static str,
    /// Cookie path.
    pub path: &'static str,
    /// SameSite value.
    pub same_site: &'static str,
    /// Whether Secure is enabled.
    pub is_secure: bool,
    /// Whether HttpOnly is enabled.
    pub is_http_only: bool,
}

impl SessionCookiePolicy {
    /// Build the default session cookie policy.
    ///
    /// # Arguments
    ///
    /// * `is_secure` - Whether Secure should be enabled.
    ///
    /// # Returns
    ///
    /// Session cookie policy matching the Python API defaults.
    pub fn new(is_secure: bool) -> Self {
        Self {
            name: SESSION_COOKIE_NAME,
            path: SESSION_COOKIE_PATH,
            same_site: SESSION_COOKIE_SAME_SITE,
            is_secure,
            is_http_only: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionCookiePolicy;

    #[test]
    fn defaults_match_python_cookie_policy() {
        let policy = SessionCookiePolicy::new(false);

        assert_eq!(policy.name, "litradar_session");
        assert_eq!(policy.path, "/");
        assert_eq!(policy.same_site, "lax");
        assert!(!policy.is_secure);
        assert!(policy.is_http_only);
    }
}
