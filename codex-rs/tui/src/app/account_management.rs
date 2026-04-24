//! Super Codex: multi-account management handlers.
//!
//! The TUI-owned multi-account flows — `/accounts` switch, `/addaccount`
//! reload hint, `/removeaccount` delete-and-optionally-rotate — are
//! driven here by calling into the shared `Arc<AuthManager>` that
//! `AppServerSession::auth_manager()` exposes from the embedded
//! in-process app-server runtime. The 0.124 RPC surface has no typed
//! method for these operations, and adding new RPC variants across
//! the protocol / server / client / TUI layers is much more
//! boilerplate than the Arc-sharing approach this module takes.

use super::*;

impl App {
    /// Super Codex: re-read the active account from the app-server and
    /// push it into the chat widget so the `/status` card, the session
    /// info, and any connector gating reflect the current `auth.json`
    /// instead of the boot-time snapshot. Silently no-op in remote
    /// app-server mode (the refresh is a UX nicety, not correctness
    /// critical).
    pub(super) async fn refresh_status_account_from_server(
        &mut self,
        app_server: &mut AppServerSession,
    ) {
        match app_server.read_status_account_state().await {
            Ok((display, plan_type, has_chatgpt)) => {
                self.chat_widget
                    .update_account_state(display, plan_type, has_chatgpt);
            }
            Err(err) => {
                tracing::warn!(
                    "Super Codex: failed to refresh account status after auth change: {err}"
                );
            }
        }
    }

    /// Super Codex: handle `/accounts` switch-to-saved by driving the
    /// shared AuthManager directly.
    pub(super) async fn handle_switch_chatgpt_account(
        &mut self,
        app_server: &mut AppServerSession,
        account_id: String,
    ) {
        let Some(auth_manager) = app_server.auth_manager() else {
            self.chat_widget.add_error_message(
                "Account switching is not available in remote app-server mode.".to_string(),
            );
            return;
        };

        let codex_home = self.chat_widget.config_ref().codex_home.to_path_buf();
        let store_mode = self
            .chat_widget
            .config_ref()
            .cli_auth_credentials_store_mode;

        match auth_manager
            .switch_to_saved_chatgpt_account(&codex_home, store_mode, &account_id)
            .await
        {
            Ok(saved) => {
                self.chat_widget
                    .add_info_message(format!("Switched to {}.", saved.label), /*hint*/ None);
                self.refresh_status_account_from_server(app_server).await;
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    self.chat_widget
                        .add_error_message(format!("Saved account not found: {account_id}"));
                } else {
                    self.chat_widget
                        .add_error_message(format!("Failed to switch account: {err}"));
                }
            }
        }
    }

    /// Super Codex: handle the post-login `/addaccount` hint by
    /// reloading the shared AuthManager from disk. Best-effort — in
    /// remote mode we silently skip because the auth.json we wrote
    /// isn't visible to the remote server anyway.
    pub(super) async fn handle_reload_auth(&mut self, app_server: &mut AppServerSession) {
        let Some(auth_manager) = app_server.auth_manager() else {
            tracing::debug!(
                "Super Codex: ignoring ReloadAuthRequested in remote app-server mode"
            );
            return;
        };

        let changed = auth_manager.reload();
        tracing::debug!(
            changed,
            "Super Codex: AuthManager reloaded after /addaccount"
        );
        self.refresh_status_account_from_server(app_server).await;
    }

    /// Super Codex: handle `/removeaccount` end-to-end. Deletes the
    /// registry entry and, if it was the active account, rotates to
    /// the next available saved one so subsequent turns keep working.
    pub(super) async fn handle_remove_chatgpt_account(
        &mut self,
        app_server: &mut AppServerSession,
        account_id: String,
        label: String,
    ) {
        let codex_home = self.chat_widget.config_ref().codex_home.to_path_buf();
        let store_mode = self
            .chat_widget
            .config_ref()
            .cli_auth_credentials_store_mode;

        // Capture whether this entry is the currently active account
        // BEFORE we delete it, so we know whether to rotate.
        let was_active =
            match codex_login::current_saved_chatgpt_account_id(&codex_home, store_mode) {
                Ok(current) => current.as_deref() == Some(account_id.as_str()),
                Err(err) => {
                    tracing::warn!(
                        "Super Codex: failed to read current saved account id: {err}"
                    );
                    false
                }
            };

        match codex_login::remove_saved_chatgpt_account(&codex_home, &account_id) {
            Ok(true) => {}
            Ok(false) => {
                self.chat_widget
                    .add_error_message(format!("Saved account not found: {label}"));
                return;
            }
            Err(err) => {
                self.chat_widget
                    .add_error_message(format!("Failed to remove account: {err}"));
                return;
            }
        }

        if !was_active {
            self.chat_widget
                .add_info_message(format!("Removed {label}."), /*hint*/ None);
            return;
        }

        // The removed account was active — pick the first remaining
        // saved account (if any) and switch to it so the session
        // keeps working without a manual `/accounts` pass.
        let remaining = match codex_login::list_saved_chatgpt_accounts(&codex_home) {
            Ok(list) => list,
            Err(err) => {
                self.chat_widget.add_info_message(
                    format!(
                        "Removed {label}, but failed to list remaining saved accounts: {err}"
                    ),
                    /*hint*/ None,
                );
                return;
            }
        };

        let Some(next) = remaining.into_iter().next() else {
            self.chat_widget.add_info_message(
                format!(
                    "Removed {label}. No other saved accounts; run /addaccount to add a new one."
                ),
                /*hint*/ None,
            );
            return;
        };

        let Some(auth_manager) = app_server.auth_manager() else {
            self.chat_widget.add_info_message(
                format!(
                    "Removed {label}. Account switching is not available in remote mode; \
                     run /accounts once the app-server is local to continue."
                ),
                /*hint*/ None,
            );
            return;
        };

        match auth_manager
            .switch_to_saved_chatgpt_account(&codex_home, store_mode, &next.id)
            .await
        {
            Ok(saved) => {
                self.chat_widget.add_info_message(
                    format!("Removed {label}. Switched to {}.", saved.label),
                    /*hint*/ None,
                );
                self.refresh_status_account_from_server(app_server).await;
            }
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Removed {label}, but failed to switch to {}: {err}",
                    next.label
                ));
            }
        }
    }
}
