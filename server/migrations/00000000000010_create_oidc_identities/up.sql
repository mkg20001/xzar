-- OpenID Connect identities linking external providers to users
CREATE TABLE oidc_identities (
    id SERIAL PRIMARY KEY,
    -- Provider ID from config (e.g., "google", "keycloak")
    provider_id VARCHAR(64) NOT NULL,
    -- Subject claim value from the OIDC provider (unique per provider)
    subject VARCHAR(256) NOT NULL,
    -- Associated user (nullable - identity can exist before user association)
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    -- Cached values from OIDC claims (for display/debugging)
    cached_email VARCHAR(256),
    cached_name VARCHAR(256),
    -- Timestamps
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_login TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    -- Unique constraint: one subject per provider
    UNIQUE (provider_id, subject)
);

-- Index for user lookups
CREATE INDEX idx_oidc_identities_user_id ON oidc_identities(user_id);

-- Index for provider+subject lookups (covered by unique constraint, but explicit for clarity)
CREATE INDEX idx_oidc_identities_provider_subject ON oidc_identities(provider_id, subject);
