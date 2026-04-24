use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

use chrono::DateTime;
use chrono::Utc;
use codex_app_server_protocol::AuthMode as ApiAuthMode;
use codex_config::types::AuthCredentialsStoreMode;
use serde::Deserialize;
use serde::Serialize;

use super::AuthDotJson;
use super::load_auth_dot_json;
use super::save_auth;

const ACCOUNTS_FILE_NAME: &str = "accounts.json";
const ACCOUNTS_FILE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SavedChatgptAccount {
    pub id: String,
    pub label: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub added_at: DateTime<Utc>,
    pub auth: AuthDotJson,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct SavedChatgptAccountsFile {
    version: u32,
    accounts: Vec<SavedChatgptAccount>,
}

impl Default for SavedChatgptAccountsFile {
    fn default() -> Self {
        Self {
            version: ACCOUNTS_FILE_VERSION,
            accounts: Vec::new(),
        }
    }
}

fn accounts_file(codex_home: &Path) -> PathBuf {
    codex_home.join(ACCOUNTS_FILE_NAME)
}

fn load_accounts_file(codex_home: &Path) -> std::io::Result<SavedChatgptAccountsFile> {
    let path = accounts_file(codex_home);
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SavedChatgptAccountsFile::default());
        }
        Err(err) => return Err(err),
    };

    let mut parsed: SavedChatgptAccountsFile = serde_json::from_str(&contents).map_err(|err| {
        std::io::Error::other(format!("failed to parse {} as JSON: {err}", path.display()))
    })?;
    if parsed.version == 0 {
        parsed.version = ACCOUNTS_FILE_VERSION;
    }
    Ok(parsed)
}

fn save_accounts_file(codex_home: &Path, file: &SavedChatgptAccountsFile) -> std::io::Result<()> {
    let path = accounts_file(codex_home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json_data = serde_json::to_string_pretty(file)?;
    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file_handle = options.open(path)?;
    file_handle.write_all(json_data.as_bytes())?;
    file_handle.flush()?;
    Ok(())
}

fn account_metadata(auth: &AuthDotJson) -> (Option<String>, Option<String>) {
    let tokens = auth.tokens.as_ref();
    let account_id = tokens
        .and_then(|tokens| tokens.account_id.clone())
        .or_else(|| tokens.and_then(|tokens| tokens.id_token.chatgpt_account_id.clone()));
    let email = tokens.and_then(|tokens| tokens.id_token.email.clone());
    (account_id, email)
}

fn account_label(auth: &AuthDotJson) -> String {
    let (account_id, email) = account_metadata(auth);
    match (email, account_id) {
        (Some(email), Some(account_id)) => format!("{email} ({account_id})"),
        (Some(email), None) => email,
        (None, Some(account_id)) => format!("workspace {account_id}"),
        (None, None) => "ChatGPT account".to_string(),
    }
}

pub fn account_registry_id_for_auth(auth: &AuthDotJson) -> Option<String> {
    let tokens = auth.tokens.as_ref()?;
    if let Some(account_id) = tokens
        .account_id
        .as_deref()
        .or(tokens.id_token.chatgpt_account_id.as_deref())
    {
        return Some(format!("account:{account_id}"));
    }
    if let Some(user_id) = tokens.id_token.chatgpt_user_id.as_deref() {
        return Some(format!("user:{user_id}"));
    }
    tokens
        .id_token
        .email
        .as_deref()
        .map(|email| format!("email:{}", email.to_lowercase()))
}

pub fn list_saved_chatgpt_accounts(codex_home: &Path) -> std::io::Result<Vec<SavedChatgptAccount>> {
    Ok(load_accounts_file(codex_home)?.accounts)
}

pub fn upsert_saved_chatgpt_account(
    codex_home: &Path,
    auth: &AuthDotJson,
) -> std::io::Result<SavedChatgptAccount> {
    if auth.resolved_mode() == ApiAuthMode::ApiKey {
        return Err(std::io::Error::other(
            "cannot store API key auth in ChatGPT accounts registry",
        ));
    }

    let Some(id) = account_registry_id_for_auth(auth) else {
        return Err(std::io::Error::other(
            "could not infer a stable account id from current auth tokens",
        ));
    };
    let mut file = load_accounts_file(codex_home)?;
    let now = Utc::now();
    let (account_id, email) = account_metadata(auth);

    let mut entry = SavedChatgptAccount {
        id: id.clone(),
        label: account_label(auth),
        account_id,
        email,
        added_at: now,
        auth: auth.clone(),
    };

    if let Some(existing) = file.accounts.iter_mut().find(|existing| existing.id == id) {
        entry.added_at = existing.added_at;
        *existing = entry.clone();
    } else {
        file.accounts.push(entry.clone());
    }

    save_accounts_file(codex_home, &file)?;
    Ok(entry)
}

/// Update the registry entry for the account identified by `auth` with
/// the freshest tokens — but **never** create a new entry if one does
/// not exist.
///
/// Used by the continuous-mirror hook on `AuthManager` so a just-removed
/// account is not silently re-added to `accounts.json` while the in-
/// memory cache still carries its auth. Returns `Ok(true)` when an
/// existing entry was updated, `Ok(false)` when there was nothing to
/// mirror (no matching id, API-key auth, or missing registry file).
pub fn update_saved_chatgpt_account_if_exists(
    codex_home: &Path,
    auth: &AuthDotJson,
) -> std::io::Result<bool> {
    if auth.resolved_mode() == ApiAuthMode::ApiKey {
        return Ok(false);
    }
    let Some(id) = account_registry_id_for_auth(auth) else {
        return Ok(false);
    };
    let mut file = load_accounts_file(codex_home)?;
    let Some(existing) = file.accounts.iter_mut().find(|entry| entry.id == id) else {
        return Ok(false);
    };
    let (account_id, email) = account_metadata(auth);
    existing.label = account_label(auth);
    existing.account_id = account_id;
    existing.email = email;
    existing.auth = auth.clone();
    save_accounts_file(codex_home, &file)?;
    Ok(true)
}

pub fn upsert_active_chatgpt_account(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Option<SavedChatgptAccount>> {
    let Some(auth) = load_auth_dot_json(codex_home, auth_credentials_store_mode)? else {
        return Ok(None);
    };
    if auth.resolved_mode() == ApiAuthMode::ApiKey {
        return Ok(None);
    }
    upsert_saved_chatgpt_account(codex_home, &auth).map(Some)
}

pub fn current_saved_chatgpt_account_id(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<Option<String>> {
    let Some(auth) = load_auth_dot_json(codex_home, auth_credentials_store_mode)? else {
        return Ok(None);
    };
    Ok(account_registry_id_for_auth(&auth))
}

pub fn remove_saved_chatgpt_account(codex_home: &Path, account_id: &str) -> std::io::Result<bool> {
    let mut file = load_accounts_file(codex_home)?;
    let previous_len = file.accounts.len();
    file.accounts.retain(|entry| entry.id != account_id);
    if file.accounts.len() == previous_len {
        return Ok(false);
    }
    save_accounts_file(codex_home, &file)?;
    Ok(true)
}

pub fn switch_active_chatgpt_account(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    account_id: &str,
) -> std::io::Result<SavedChatgptAccount> {
    let file = load_accounts_file(codex_home)?;
    let Some(entry) = file
        .accounts
        .iter()
        .find(|entry| entry.id == account_id)
        .cloned()
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("saved account not found: {account_id}"),
        ));
    };

    save_auth(codex_home, &entry.auth, auth_credentials_store_mode)?;
    Ok(entry)
}

pub fn rotate_to_next_saved_chatgpt_account(
    codex_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    exhausted_account_ids: &HashSet<String>,
) -> std::io::Result<Option<SavedChatgptAccount>> {
    let file = load_accounts_file(codex_home)?;
    if file.accounts.is_empty() {
        return Ok(None);
    }

    let current_id = current_saved_chatgpt_account_id(codex_home, auth_credentials_store_mode)?;
    let next = file.accounts.into_iter().find(|entry| {
        if exhausted_account_ids.contains(&entry.id) {
            return false;
        }
        match current_id.as_ref() {
            Some(current_id) => &entry.id != current_id,
            None => true,
        }
    });

    if let Some(entry) = next {
        save_auth(codex_home, &entry.auth, auth_credentials_store_mode)?;
        return Ok(Some(entry));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_data::IdTokenInfo;
    use crate::token_data::TokenData;
    use base64::Engine;
    use serde_json::json;
    use tempfile::tempdir;

    fn fake_jwt(email: &str, account_id: &str) -> String {
        let header = json!({ "alg": "none", "typ": "JWT" });
        let payload = json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "chatgpt_user_id": format!("user-{account_id}"),
                "chatgpt_plan_type": "pro",
            }
        });
        let encode = |value: serde_json::Value| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&value).expect("json should serialize"))
        };
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
        format!("{}.{}.{}", encode(header), encode(payload), signature)
    }

    fn sample_auth(email: &str, account_id: &str) -> AuthDotJson {
        AuthDotJson {
            auth_mode: Some(ApiAuthMode::Chatgpt),
            openai_api_key: None,
            tokens: Some(TokenData {
                id_token: IdTokenInfo {
                    email: Some(email.to_string()),
                    chatgpt_plan_type: None,
                    chatgpt_user_id: Some(format!("user-{account_id}")),
                    chatgpt_account_id: Some(account_id.to_string()),
                    chatgpt_account_is_fedramp: false,
                    raw_jwt: fake_jwt(email, account_id),
                },
                access_token: format!("access-{account_id}"),
                refresh_token: format!("refresh-{account_id}"),
                account_id: Some(account_id.to_string()),
            }),
            last_refresh: Some(Utc::now()),
            agent_identity: None,
        }
    }

    #[test]
    fn upsert_and_list_saved_accounts() {
        let dir = tempdir().unwrap();
        let first = sample_auth("first@example.com", "acc_first");
        let second = sample_auth("second@example.com", "acc_second");

        let saved_first = upsert_saved_chatgpt_account(dir.path(), &first).unwrap();
        let saved_second = upsert_saved_chatgpt_account(dir.path(), &second).unwrap();

        let listed = list_saved_chatgpt_accounts(dir.path()).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, saved_first.id);
        assert_eq!(listed[1].id, saved_second.id);
    }

    #[test]
    fn switch_active_account_updates_auth_storage() {
        let dir = tempdir().unwrap();
        let first = sample_auth("first@example.com", "acc_first");
        let second = sample_auth("second@example.com", "acc_second");
        upsert_saved_chatgpt_account(dir.path(), &first).unwrap();
        let saved_second = upsert_saved_chatgpt_account(dir.path(), &second).unwrap();

        switch_active_chatgpt_account(dir.path(), AuthCredentialsStoreMode::File, &saved_second.id)
            .unwrap();

        let current = load_auth_dot_json(dir.path(), AuthCredentialsStoreMode::File)
            .unwrap()
            .unwrap();
        let current_id = account_registry_id_for_auth(&current).unwrap();
        assert_eq!(current_id, saved_second.id);
    }

    #[test]
    fn rotate_skips_exhausted_accounts() {
        let dir = tempdir().unwrap();
        let first = sample_auth("first@example.com", "acc_first");
        let second = sample_auth("second@example.com", "acc_second");
        let third = sample_auth("third@example.com", "acc_third");
        let saved_first = upsert_saved_chatgpt_account(dir.path(), &first).unwrap();
        let saved_second = upsert_saved_chatgpt_account(dir.path(), &second).unwrap();
        let saved_third = upsert_saved_chatgpt_account(dir.path(), &third).unwrap();

        save_auth(dir.path(), &first, AuthCredentialsStoreMode::File).unwrap();

        let exhausted = HashSet::from([saved_first.id.clone(), saved_second.id.clone()]);
        let rotated = rotate_to_next_saved_chatgpt_account(
            dir.path(),
            AuthCredentialsStoreMode::File,
            &exhausted,
        )
        .unwrap()
        .unwrap();
        assert_eq!(rotated.id, saved_third.id);
    }

    #[test]
    fn remove_account_from_registry() {
        let dir = tempdir().unwrap();
        let first = sample_auth("first@example.com", "acc_first");
        let saved = upsert_saved_chatgpt_account(dir.path(), &first).unwrap();

        assert!(remove_saved_chatgpt_account(dir.path(), &saved.id).unwrap());
        assert!(!remove_saved_chatgpt_account(dir.path(), &saved.id).unwrap());
        assert!(list_saved_chatgpt_accounts(dir.path()).unwrap().is_empty());
    }

    /// Regression: the continuous-mirror hook must **not** re-insert
    /// an account that was removed from the registry. Otherwise
    /// `/removeaccount` followed by `switch_to_saved_chatgpt_account`
    /// (which calls the mirror before overwriting auth.json) would
    /// put the just-deleted entry back and leave the user cycling
    /// between ghost entries.
    #[test]
    fn update_if_exists_does_not_reinsert_removed_account() {
        let dir = tempdir().unwrap();
        let first = sample_auth("first@example.com", "acc_first");
        let second = sample_auth("second@example.com", "acc_second");

        // Two accounts saved, then we remove `first`.
        let saved_first = upsert_saved_chatgpt_account(dir.path(), &first).unwrap();
        upsert_saved_chatgpt_account(dir.path(), &second).unwrap();
        assert!(remove_saved_chatgpt_account(dir.path(), &saved_first.id).unwrap());

        // Mirror-hook call with `first`'s still-cached auth: must be a
        // no-op because there is no matching entry anymore.
        let updated = update_saved_chatgpt_account_if_exists(dir.path(), &first).unwrap();
        assert!(!updated);
        let listed = list_saved_chatgpt_accounts(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_ne!(listed[0].id, saved_first.id);

        // Same hook with `second`'s auth (still in registry) rewrites
        // the entry's tokens in place; id + added_at unchanged.
        let mut refreshed_second = second.clone();
        if let Some(tokens) = refreshed_second.tokens.as_mut() {
            tokens.access_token = "rotated-access".to_string();
            tokens.refresh_token = "rotated-refresh".to_string();
        }
        let updated = update_saved_chatgpt_account_if_exists(dir.path(), &refreshed_second).unwrap();
        assert!(updated);
        let listed = list_saved_chatgpt_accounts(dir.path()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0]
                .auth
                .tokens
                .as_ref()
                .map(|t| t.refresh_token.clone()),
            Some("rotated-refresh".to_string())
        );
    }
}
