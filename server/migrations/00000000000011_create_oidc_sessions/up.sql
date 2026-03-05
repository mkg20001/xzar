-- Temporary OIDC authentication sessions (for OAuth2 state/nonce)
CREATE TABLE oidc_sessions (
    id SERIAL PRIMARY KEY,
    -- State parameter (CSRF protection)
    state VARCHAR(128) NOT NULL UNIQUE,
    -- Provider ID this session is for
    provider_id VARCHAR(64) NOT NULL,
    -- Nonce for ID token validation
    nonce VARCHAR(128) NOT NULL,
    -- Optional redirect URL after successful auth
    redirect_url VARCHAR(1024),
    -- Expiration (sessions are short-lived)
    expires TIMESTAMP NOT NULL,
    -- Creation timestamp
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Index for state lookups
CREATE INDEX idx_oidc_sessions_state ON oidc_sessions(state);

-- Index for cleanup of expired sessions
CREATE INDEX idx_oidc_sessions_expires ON oidc_sessions(expires);
