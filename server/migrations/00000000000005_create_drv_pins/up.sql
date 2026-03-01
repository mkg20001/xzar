CREATE TABLE drv_pins (
    drv_id VARCHAR(32) NOT NULL REFERENCES drvs(drv_id) ON UPDATE CASCADE ON DELETE CASCADE,
    pin_id INTEGER NOT NULL REFERENCES pins(id) ON UPDATE CASCADE ON DELETE CASCADE,
    PRIMARY KEY (drv_id, pin_id)
);

CREATE INDEX idx_drv_pins_drv_id ON drv_pins(drv_id);
CREATE INDEX idx_drv_pins_pin_id ON drv_pins(pin_id);
