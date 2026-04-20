pub mod default_client;
pub mod error;
mod storage;
mod util;

mod account_registry;
mod external_bearer;
mod manager;

pub use account_registry::SavedChatgptAccount;
pub use account_registry::account_registry_id_for_auth;
pub use account_registry::current_saved_chatgpt_account_id;
pub use account_registry::list_saved_chatgpt_accounts;
pub use account_registry::remove_saved_chatgpt_account;
pub use account_registry::rotate_to_next_saved_chatgpt_account;
pub use account_registry::switch_active_chatgpt_account;
pub use account_registry::upsert_active_chatgpt_account;
pub use account_registry::upsert_saved_chatgpt_account;
pub use error::RefreshTokenFailedError;
pub use error::RefreshTokenFailedReason;
pub use manager::*;
