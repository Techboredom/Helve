-- Extra GIDs (Kubernetes pod securityContext supplementalGroups) added to a
-- user's launches alongside runAsUser/runAsGroup/fsGroup (uid/gid above) —
-- the POSIX-ACL/NFS pattern of belonging to several groups, each granting
-- access to a different share, rather than just one primary GID. Empty
-- array (not NULL) means none set, so call sites can treat it as a plain
-- list rather than an Option<Vec<_>> — an empty list already means "no
-- supplemental groups" on its own.
ALTER TABLE users ADD COLUMN IF NOT EXISTS supplemental_groups INTEGER[] NOT NULL DEFAULT '{}';
