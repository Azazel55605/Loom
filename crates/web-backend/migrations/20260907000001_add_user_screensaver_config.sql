-- Ordered kiosk screensaver readings. The selection is server-owned so every
-- mobile kiosk using this account receives the same ambient presentation.
ALTER TABLE users ADD COLUMN screensaver_config TEXT;
