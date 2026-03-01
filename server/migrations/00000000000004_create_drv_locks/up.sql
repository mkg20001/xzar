CREATE TABLE drv_locks (
    drv_id VARCHAR(32) NOT NULL REFERENCES drvs(drv_id) ON UPDATE CASCADE ON DELETE CASCADE,
    lock_id INTEGER NOT NULL REFERENCES locks(id) ON UPDATE CASCADE ON DELETE CASCADE,
    PRIMARY KEY (drv_id, lock_id)
);

CREATE INDEX idx_drv_locks_drv_id ON drv_locks(drv_id);
CREATE INDEX idx_drv_locks_lock_id ON drv_locks(lock_id);
