-- Named groups (an admin-managed registry: a display name plus the real
-- GID it means) and per-user membership in them, replacing the raw GID
-- list added in 0023. A GID is meant to be shared across whichever users
-- need access to the same thing (an NFS share, say) — naming it once here,
-- rather than letting each user's assignment carry its own free-text
-- label, is the only design where everyone sharing that GID agrees on what
-- it's called.
CREATE TABLE groups (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    gid INTEGER NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ON DELETE RESTRICT (not CASCADE): deleting a group a user is still
-- assigned to would silently change what their future launches run
-- as — matching this project's general preference for refusing rather
-- than silently doing something an admin didn't explicitly ask for (see
-- delete_user's own refusal when a user still owns running Deployments).
CREATE TABLE user_groups (
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE RESTRICT,
    PRIMARY KEY (user_id, group_id)
);

ALTER TABLE users DROP COLUMN supplemental_groups;
