CREATE TABLE drvs (
    drv_id VARCHAR(32) PRIMARY KEY,
    drv_full VARCHAR NOT NULL,
    nar_hash VARCHAR NOT NULL,
    nar_size BIGINT NOT NULL,
    file_hash VARCHAR NOT NULL,
    file_size BIGINT NOT NULL,
    deriver VARCHAR,
    sig VARCHAR,
    refs VARCHAR[] NOT NULL DEFAULT '{}',
    nar_comp VARCHAR,
    nar_file VARCHAR NOT NULL,
    nar_file_storage VARCHAR NOT NULL,
    gc BOOLEAN NOT NULL DEFAULT FALSE,
    created TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_fetched TIMESTAMP
);

CREATE INDEX idx_drvs_drv_full ON drvs(drv_full);
CREATE INDEX idx_drvs_gc ON drvs(gc);
