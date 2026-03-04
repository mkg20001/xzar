CREATE TABLE tokens (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES users(id) ON DELETE CASCADE,
    token_hash VARCHAR(128) NOT NULL UNIQUE,
    is_system BOOLEAN NOT NULL DEFAULT FALSE,
    description VARCHAR(256),
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_tokens_token_hash ON tokens(token_hash);
CREATE INDEX idx_tokens_user_id ON tokens(user_id);

-- Constraint: system tokens have NULL user_id, user tokens must have user_id
ALTER TABLE tokens ADD CONSTRAINT check_token_type CHECK (
    (is_system = TRUE AND user_id IS NULL) OR
    (is_system = FALSE AND user_id IS NOT NULL)
);
