-- The project was renamed Aether -> Helve. Two seeded template `notes` values
-- name the product in text an admin reads in the Templates tab, so they have
-- to follow.
--
-- Done as a new migration rather than by editing 0006/0007 in place: sqlx
-- records a checksum per applied migration, so rewriting one that has already
-- run makes the binary refuse to start against an existing database with a
-- VersionMismatch. Old migrations are immutable once released.
--
-- Each UPDATE is scoped to the exact string those migrations wrote, so an
-- admin who has since edited the notes keeps their version instead of having
-- it silently reverted.

UPDATE templates
SET notes = 'Click "Open" on the Pods tab once it''s running — you''ll land in an already-logged-in session, no token needed. Helve''s proxy is the only way in, so this never gets a public IP.'
WHERE name = 'JupyterLab'
  AND notes = 'Click "Open" on the Pods tab once it''s running — you''ll land in an already-logged-in session, no token needed. Aether''s proxy is the only way in, so this never gets a public IP.';

UPDATE templates
SET notes = 'Runs with authentication fully disabled — click "Open" on the Pods tab once it''s running to go straight in as the "rstudio" user. Helve''s own login (and ownership check) is the only thing gating access, so this never gets a public IP.'
WHERE name = 'RStudio'
  AND notes = 'Runs with authentication fully disabled — click "Open" on the Pods tab once it''s running to go straight in as the "rstudio" user. Aether''s own login (and ownership check) is the only thing gating access, so this never gets a public IP.';
