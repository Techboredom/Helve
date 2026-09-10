-- Whether launching this template replaces /etc/passwd and /etc/group with
-- copies of the image's own plus an entry naming the launching user's
-- uid/gid/supplemental groups, via an init container (see
-- deployments.rs::identity_files_init_container). Off by default: it
-- needs the image to have a POSIX shell, which isn't true of every image
-- Helve can launch.
ALTER TABLE templates ADD COLUMN inject_identity_files BOOLEAN NOT NULL DEFAULT false;
