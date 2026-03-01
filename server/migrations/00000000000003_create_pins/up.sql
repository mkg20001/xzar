CREATE TABLE pins (
    id SERIAL PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    description VARCHAR(1024),
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires TIMESTAMP,
    abandoned BOOLEAN NOT NULL DEFAULT FALSE,
    leave_after_abandon BIGINT
);

CREATE INDEX idx_pins_name ON pins(name);
CREATE INDEX idx_pins_abandoned ON pins(abandoned);
CREATE INDEX idx_pins_expires ON pins(expires);
