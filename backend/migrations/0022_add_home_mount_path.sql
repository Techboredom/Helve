-- Where inside the container a per-user home directory gets mounted, if the
-- backend is configured for one (HOME_DRIVES_HOST_BASE_PATH or
-- HOME_DRIVES_STORAGE_CLASS) — empty means this template doesn't use one,
-- same NOT NULL DEFAULT '' convention as volume_mount_path.
ALTER TABLE templates ADD COLUMN home_mount_path TEXT NOT NULL DEFAULT '';
