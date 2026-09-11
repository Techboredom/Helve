-- LDAP/AD- and (in 0027) OIDC-provisioned accounts have no local password at
-- all, only ever authenticating against the external directory/IdP —
-- password_hash must become nullable to represent that. A NULL hash can
-- never pass verify_password's check against a real value, so local login
-- is naturally locked out for these accounts without any extra logic.
ALTER TABLE users ALTER COLUMN password_hash DROP NOT NULL;

-- Which check provisioned (or, for local accounts, simply is) this
-- account's login path — 'local', 'ldap', or 'oidc'. Read-only, shown on
-- the Users admin tab; doesn't gate anything by itself (an admin can still
-- reset an LDAP/OIDC-provisioned account's password to add a local
-- break-glass path, for instance).
ALTER TABLE users ADD COLUMN auth_source TEXT NOT NULL DEFAULT 'local';
