-- The OIDC `sub` claim: the only value the spec guarantees is stable for a
-- given account at a given issuer. Never link/match by email or username
-- instead (either can change, or be reused by a different person) except
-- for the one explicit case in oidc.rs (auto_provision=false, no existing
-- oidc_subject yet — an admin pre-created the account and it's being
-- linked for the first time).
ALTER TABLE users ADD COLUMN oidc_subject TEXT UNIQUE;

-- CSRF state + PKCE verifier for an in-flight authorization-code flow,
-- surviving the redirect out to the IdP and back. Postgres-backed rather
-- than in-process: replicaCount can be >1, and the callback can land on a
-- different pod than the one that issued the redirect.
-- `nonce` is openidconnect's own replay/injection protection, separate
-- from `state`'s CSRF protection above — the ID token carries it back as a
-- claim, checked against this stored value at verification time.
CREATE TABLE oidc_flow_state (
    state TEXT PRIMARY KEY,
    pkce_verifier TEXT NOT NULL,
    nonce TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
