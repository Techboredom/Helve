//! LDAP/Active Directory login — tried by `auth::login` whenever the local
//! password check fails and `AppState::ldap` is configured. Search-then-bind
//! only (not a direct bind-DN template): Helve binds as a service account to
//! find the user's DN, then opens a second connection and binds as that DN
//! with the submitted password, which is the actual credential check — a
//! successful search proves nothing about the password by itself.

use ldap3::{LdapConnAsync, LdapError, Scope, SearchEntry};

use crate::error::ApiError;
use crate::state::{AppState, LdapConfig};

/// Characters meaningful in an LDAP search filter (RFC 4515) — if the
/// submitted username contains any of these, it's rejected before it ever
/// reaches `LdapConfig::user_filter`'s `{username}` substitution, the same
/// "validate before it reaches a query-building step" instinct as
/// `validate::k8s_name` elsewhere. Filter *injection* (not just a malformed
/// filter) is the concern — `*` could turn an exact-match filter into a
/// wildcard matching an unintended entry, and `)`/`(` could inject extra
/// filter clauses entirely.
fn contains_filter_metacharacters(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '*' | '(' | ')' | '\\' | '\0'))
}

/// The actual directory round trip: search for `username`'s DN as the
/// service account, then verify the password by binding as that DN.
/// Returns the admin-group membership check on success (`Some(is_admin)`).
/// Returns `None` — not an error — for "no such user", "ambiguous
/// filter" (more than one match), "bad password", *and* any LDAP
/// connection/protocol failure (logged via `tracing::error!` for an
/// admin to notice, but never surfaced to the caller): from the caller's
/// point of view none of these should be distinguishable from a wrong
/// password, and a directory outage must not become a way to probe
/// which usernames exist.
async fn authenticate(config: &LdapConfig, username: &str, password: &str) -> Option<bool> {
    // An empty password binds anonymously on most directory servers,
    // which "succeeds" without checking anything at all — never let that
    // stand in for a real credential.
    if password.is_empty() || contains_filter_metacharacters(username) {
        return None;
    }

    match authenticate_inner(config, username, password).await {
        Ok(is_admin) => is_admin,
        Err(err) => {
            tracing::error!(error = %err, "LDAP authentication failed");
            None
        }
    }
}

async fn authenticate_inner(config: &LdapConfig, username: &str, password: &str) -> Result<Option<bool>, LdapError> {
    let (conn, mut service) = LdapConnAsync::new(&config.url).await?;
    ldap3::drive!(conn);
    service.simple_bind(&config.bind_dn, &config.bind_password).await?.success()?;

    let filter = config.user_filter.replace("{username}", username);
    let (entries, _res) = service.search(&config.base_dn, Scope::Subtree, &filter, vec!["dn"]).await?.success()?;
    let Some(entry) = entries.into_iter().next() else {
        let _ = service.unbind().await;
        return Ok(None);
    };
    let user_dn = SearchEntry::construct(entry).dn;

    // The actual check: bind as the found DN with the caller's password.
    let (user_conn, mut user_ldap) = LdapConnAsync::new(&config.url).await?;
    ldap3::drive!(user_conn);
    let bound = user_ldap.simple_bind(&user_dn, password).await?.success().is_ok();
    let _ = user_ldap.unbind().await;
    if !bound {
        let _ = service.unbind().await;
        return Ok(None);
    }

    let is_admin = match &config.admin_group_dn {
        Some(group_dn) => {
            let member_filter = format!("(member={})", ldap3::ldap_escape(&user_dn));
            let (matches, _res) = service.search(group_dn, Scope::Base, &member_filter, vec!["dn"]).await?.success()?;
            !matches.is_empty()
        }
        None => false,
    };
    let _ = service.unbind().await;

    Ok(Some(is_admin))
}

/// Called from `auth::login` after a local password check has already
/// failed. Returns the Helve user id to establish a session for, `None` to
/// fall through to the same generic "invalid username or password" 401
/// local login gets, or `Err(ApiError::Forbidden(_))` for the one case
/// that's a genuinely different, actionable problem: LDAP itself vouched
/// for this person, but no Helve account exists for them and
/// `auto_provision` is off.
pub async fn authenticate_and_provision(
    state: &AppState,
    config: &LdapConfig,
    username: &str,
    password: &str,
) -> Result<Option<i32>, ApiError> {
    let Some(is_admin) = authenticate(config, username, password).await else { return Ok(None) };

    let existing: Option<i32> = sqlx::query_scalar("SELECT id FROM users WHERE username = $1").bind(username).fetch_optional(&state.pg).await?;

    let user_id = match existing {
        Some(id) => id,
        None => {
            if !config.auto_provision {
                return Err(ApiError::Forbidden(format!(
                    "\"{username}\" authenticated against LDAP, but no Helve account exists for them — contact an admin"
                )));
            }
            let role = if is_admin { "admin" } else { "user" };
            sqlx::query_scalar("INSERT INTO users (username, role, auth_source) VALUES ($1, $2, 'ldap') RETURNING id")
                .bind(username)
                .bind(role)
                .fetch_one(&state.pg)
                .await?
        }
    };

    // The directory is the source of truth once group mapping is on: this
    // overwrites a role manually set from the Users tab on every login,
    // not just at first provisioning.
    if config.admin_group_dn.is_some() {
        let role = if is_admin { "admin" } else { "user" };
        sqlx::query("UPDATE users SET role = $1 WHERE id = $2").bind(role).bind(user_id).execute(&state.pg).await?;
    }

    Ok(Some(user_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_filter_metacharacters() {
        for username in ["*", "admin)(uid=*", "a\\b", "a(b", "a)b"] {
            assert!(contains_filter_metacharacters(username), "should reject {username:?}");
        }
        for username in ["alice", "bob.smith", "a-b_c123"] {
            assert!(!contains_filter_metacharacters(username), "should accept {username:?}");
        }
    }

}
