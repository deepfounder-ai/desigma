//! Where the OpenRouter key lives: the OS credential store (Windows Credential Manager, macOS
//! Keychain, the Secret Service on Linux), never a file. `OPENROUTER_API_KEY` in the environment
//! wins over the stored key, for machines without a credential store.

const ENV: &str = "OPENROUTER_API_KEY";

/// The key to use, if there is one.
pub fn load() -> Option<String> {
    std::env::var(ENV).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).or_else(store::get)
}

/// The key comes from the environment, so the dialog cannot change it.
pub fn from_env() -> bool {
    std::env::var(ENV).is_ok_and(|k| !k.trim().is_empty())
}

/// Stores `key`; an empty key forgets the stored one.
pub fn save(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        store::delete()
    } else {
        store::set(key)
    }
}

#[cfg(all(not(test), any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod store {
    const SERVICE: &str = "desigma";
    const ACCOUNT: &str = "openrouter-api-key";

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(explain)
    }

    fn explain(e: keyring::Error) -> String {
        format!("The system keychain is not available ({e}). Set the OPENROUTER_API_KEY environment variable instead.")
    }

    pub fn get() -> Option<String> {
        entry().ok()?.get_password().ok().filter(|k: &String| !k.is_empty())
    }

    pub fn set(key: &str) -> Result<(), String> {
        entry()?.set_password(key).map_err(explain)
    }

    pub fn delete() -> Result<(), String> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(explain(e)),
        }
    }
}

#[cfg(all(not(test), not(any(target_os = "windows", target_os = "macos", target_os = "linux"))))]
mod store {
    pub fn get() -> Option<String> {
        None
    }
    pub fn set(_: &str) -> Result<(), String> {
        Err("No system keychain on this platform. Set the OPENROUTER_API_KEY environment variable instead.".into())
    }
    pub fn delete() -> Result<(), String> {
        Ok(())
    }
}

/// Tests never touch the real credential store.
#[cfg(test)]
mod store {
    use std::cell::RefCell;
    thread_local! {
        static KEY: RefCell<Option<String>> = const { RefCell::new(None) };
    }
    pub fn get() -> Option<String> {
        KEY.with(|k| k.borrow().clone())
    }
    pub fn set(key: &str) -> Result<(), String> {
        KEY.with(|k| *k.borrow_mut() = Some(key.to_string()));
        Ok(())
    }
    pub fn delete() -> Result<(), String> {
        KEY.with(|k| *k.borrow_mut() = None);
        Ok(())
    }
}
