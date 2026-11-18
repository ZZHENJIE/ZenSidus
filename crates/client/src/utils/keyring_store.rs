//! OS credential storage for the relay token: it lives in the platform
//! credential manager (Windows Credential Manager / DBUS Secret Service) and
//! never in `settings.json`. (Formerly: the Alpaca key id/secret and the
//! Finviz Elite token — both retired when the data layer moved to the
//! relay server, which holds those credentials now.)
//!
//! The flow works without any dedicated UI: the settings struct keeps the
//! token in memory (the input field keeps working as usual), the file save
//! scrubs it out, and the settings flush mirrors the in-memory value into
//! the store ([`sync`]). At startup [`restore_into`] pulls it back from the
//! store into the (empty) settings field.

use keyring::Entry;

use crate::utils::settings::Settings;

/// The keyring service all entries share.
const SERVICE: &str = "ZenSidus";

/// The secret the app stores, with its entry name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Secrets {
    pub relay_token: String,
}

impl Secrets {
    /// The value currently in effect (the in-memory settings).
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            relay_token: settings.relay.token.clone(),
        }
    }
}

#[derive(Clone, Copy)]
enum Which {
    RelayToken,
}

impl Which {
    const ALL: [Which; 1] = [Which::RelayToken];

    fn name(self) -> &'static str {
        match self {
            Which::RelayToken => "relay-token",
        }
    }
}

fn entry(which: Which) -> Result<Entry, keyring::Error> {
    Entry::new(SERVICE, which.name())
}

/// Fill an empty settings field from the credential store. Called once at
/// startup, before the relay client starts: a settings file written by the
/// sanitized save carries an empty token field, and this restores the real
/// value into memory. A field the file does provide (a pre-migration
/// plaintext file, or a value typed in this session) is left untouched.
pub fn restore_into(settings: &mut Settings) {
    for which in Which::ALL {
        let value = match which {
            Which::RelayToken => &mut settings.relay.token,
        };
        if !value.is_empty() {
            continue;
        }
        match entry(which).and_then(|entry| entry.get_password()) {
            Ok(stored) => *value = stored,
            // No entry yet (first run) is normal; anything else is logged.
            Err(keyring::Error::NoEntry) => {}
            Err(err) => tracing::error!(
                "{}",
                crate::tr!(
                    "app.log.keyring_read_failed",
                    entry = which.name().to_string(),
                    err = err.to_string()
                )
            ),
        }
    }
}

/// Mirror the in-memory secret into the credential store: write a changed
/// non-empty value, delete the entry when cleared. Returns the values that
/// were written so a caller can decide whether to re-sync later.
pub fn sync(current: &Secrets, synced: &mut Secrets) -> anyhow::Result<()> {
    for which in Which::ALL {
        let value = match which {
            Which::RelayToken => current.relay_token.as_str(),
        };
        let stored = match which {
            Which::RelayToken => synced.relay_token.as_str(),
        };
        if value == stored {
            continue;
        }
        let entry = entry(which)?;
        if value.is_empty() {
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(err) => return Err(err.into()),
            }
        } else {
            entry.set_password(value)?;
        }
    }
    *synced = current.clone();
    Ok(())
}
